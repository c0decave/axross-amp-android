package io.github.c0decave.axrossamp;

import android.app.Activity;
import android.content.BroadcastReceiver;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.database.Cursor;
import android.graphics.Insets;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.net.Uri;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;
import android.util.Log;
import android.view.DisplayCutout;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowManager;

import java.util.ArrayDeque;
import java.util.Locale;

/**
 * What the Rust side cannot do through the NDK: the Storage Access
 * Framework's pickers, reading content:// documents, the window's insets,
 * audio focus. One way in ({@link #call}) and one way out ({@link #event}),
 * each a single JNI signature; the numbers are the same as in
 * src/platform/mod.rs.
 *
 * Step 2 adds here: the playback service's start and stop, and a
 * MediaSession whose callbacks arrive as EVENT_MEDIA.
 */
final class Bridge {
    private static final String TAG = "axrossamp";

    // Rust -> Java (call)
    static final int CALL_PICK_FOLDER = 1;
    static final int CALL_PICK_FILES = 2;
    static final int CALL_OPEN_FD = 3;
    static final int CALL_REQUEST_FOCUS = 4;
    static final int CALL_ABANDON_FOCUS = 5;
    static final int CALL_LEAVE = 6;

    // Java -> Rust (event)
    static final int EVENT_INSETS = 1;
    static final int EVENT_PICKED = 2;
    static final int EVENT_LIFECYCLE = 3;
    static final int EVENT_AUDIO_FOCUS = 4;
    static final int EVENT_NOISY = 5;
    /** Reserved for step 2 (MediaSession callbacks): a = the command. */
    static final int EVENT_MEDIA = 6;

    private static final int REQUEST_FOLDER = 4201;
    private static final int REQUEST_FILES = 4202;

    /** A folder's tracks: no more than this many, no deeper than this. */
    private static final int MAX_TRACKS = 5000;
    private static final int MAX_DEPTH = 12;

    private static native void nativeEvent(int kind, int a, int b, int c, int d, String text);

    /** An event for the Rust side; never throws (a missing library is logged). */
    static void event(int kind, int a, int b, int c, int d, String text) {
        try {
            nativeEvent(kind, a, b, c, d, text);
        } catch (UnsatisfiedLinkError e) {
            Log.e(TAG, "native event " + kind + " not delivered", e);
        }
    }

    private final Activity activity;
    private AudioFocusRequest focusRequest;
    private boolean noisyRegistered;

    private final AudioManager.OnAudioFocusChangeListener focusListener =
            change -> event(EVENT_AUDIO_FOCUS, change, 0, 0, 0, null);

    private final BroadcastReceiver noisyReceiver = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent intent) {
            if (AudioManager.ACTION_AUDIO_BECOMING_NOISY.equals(intent.getAction())) {
                event(EVENT_NOISY, 0, 0, 0, 0, null);
            }
        }
    };

    Bridge(Activity activity) {
        this.activity = activity;
    }

    // -- the window ---------------------------------------------------------------------------

    /** The picture reaches under the system bars and around a cutout. */
    void edgeToEdge() {
        Window window = activity.getWindow();
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false);
        }
        if (Build.VERSION.SDK_INT >= 28) {
            WindowManager.LayoutParams attributes = window.getAttributes();
            attributes.layoutInDisplayCutoutMode = Build.VERSION.SDK_INT >= 30
                    ? WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
                    : WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            window.setAttributes(attributes);
        }
    }

    /**
     * The safe insets (system bars and cutout, in pixels) to the Rust side,
     * whenever they change: the native surface is the whole window.
     */
    @SuppressWarnings("deprecation")
    void watchInsets() {
        View decor = activity.getWindow().getDecorView();
        decor.setOnApplyWindowInsetsListener((view, insets) -> {
            int left;
            int top;
            int right;
            int bottom;
            if (Build.VERSION.SDK_INT >= 30) {
                Insets i = insets.getInsets(
                        WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                left = i.left;
                top = i.top;
                right = i.right;
                bottom = i.bottom;
            } else {
                left = insets.getSystemWindowInsetLeft();
                top = insets.getSystemWindowInsetTop();
                right = insets.getSystemWindowInsetRight();
                bottom = insets.getSystemWindowInsetBottom();
                if (Build.VERSION.SDK_INT >= 28) {
                    DisplayCutout cutout = insets.getDisplayCutout();
                    if (cutout != null) {
                        left = Math.max(left, cutout.getSafeInsetLeft());
                        top = Math.max(top, cutout.getSafeInsetTop());
                        right = Math.max(right, cutout.getSafeInsetRight());
                        bottom = Math.max(bottom, cutout.getSafeInsetBottom());
                    }
                }
            }
            event(EVENT_INSETS, left, top, right, bottom, null);
            return view.onApplyWindowInsets(insets);
        });
        decor.requestApplyInsets();
    }

    // -- calls from Rust ----------------------------------------------------------------------

    /**
     * One entry for everything the Rust side asks for (from any thread).
     * Returns a file descriptor for CALL_OPEN_FD (the caller owns it; -1:
     * it could not be opened), 1 or 0 for CALL_REQUEST_FOCUS, else 0.
     */
    int call(int what, String arg) {
        try {
            switch (what) {
                case CALL_PICK_FOLDER:
                    activity.runOnUiThread(this::pickFolder);
                    return 0;
                case CALL_PICK_FILES:
                    activity.runOnUiThread(this::pickFiles);
                    return 0;
                case CALL_OPEN_FD:
                    return openFd(arg);
                case CALL_REQUEST_FOCUS:
                    return requestFocus() ? 1 : 0;
                case CALL_ABANDON_FOCUS:
                    abandonFocus();
                    return 0;
                case CALL_LEAVE:
                    activity.runOnUiThread(() -> activity.moveTaskToBack(true));
                    return 0;
                default:
                    Log.w(TAG, "unknown call " + what);
                    return -1;
            }
        } catch (RuntimeException e) {
            Log.e(TAG, "call " + what + " failed", e);
            return -1;
        }
    }

    private void pickFolder() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        start(intent, REQUEST_FOLDER);
    }

    private void pickFiles() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("audio/*");
        intent.putExtra(Intent.EXTRA_MIME_TYPES, new String[] {"audio/*", "application/ogg"});
        intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        start(intent, REQUEST_FILES);
    }

    private void start(Intent intent, int request) {
        try {
            activity.startActivityForResult(intent, request);
        } catch (RuntimeException e) {
            // (no documents app on this device)
            Log.e(TAG, "no picker", e);
            event(EVENT_PICKED, -1, 0, 0, 0, "");
        }
    }

    private int openFd(String uri) {
        if (uri == null) {
            return -1;
        }
        try {
            ParcelFileDescriptor pfd =
                    activity.getContentResolver().openFileDescriptor(Uri.parse(uri), "r");
            return pfd == null ? -1 : pfd.detachFd();
        } catch (Exception e) {
            Log.w(TAG, "cannot open " + uri + ": " + e);
            return -1;
        }
    }

    // -- the pickers' answers -----------------------------------------------------------------

    void onActivityResult(int request, int result, Intent data) {
        if (request != REQUEST_FOLDER && request != REQUEST_FILES) {
            return;
        }
        if (result != Activity.RESULT_OK || data == null) {
            // (a = 0: nothing chosen)
            event(EVENT_PICKED, 0, 0, 0, 0, "");
            return;
        }
        // (a folder is read off the main thread: it may hold thousands)
        Thread reader = new Thread(() -> {
            StringBuilder out = new StringBuilder();
            int count = 0;
            try {
                if (request == REQUEST_FOLDER) {
                    count = listTree(data.getData(), out);
                } else {
                    count = listFiles(data, out);
                }
            } catch (RuntimeException e) {
                Log.e(TAG, "reading the choice failed", e);
            }
            event(EVENT_PICKED, count, 0, 0, 0, out.toString());
        }, "amp-picked");
        reader.start();
    }

    /** One line a track: uri, tab, folder ("" at the top), tab, name. */
    private static void line(StringBuilder out, Uri uri, String folder, String name) {
        out.append(uri).append('\t').append(clean(folder)).append('\t').append(clean(name))
                .append('\n');
    }

    private static String clean(String text) {
        return text == null ? "" : text.replace('\t', ' ').replace('\n', ' ').replace('\r', ' ');
    }

    private int listFiles(Intent data, StringBuilder out) {
        int count = 0;
        ClipData clip = data.getClipData();
        if (clip != null) {
            for (int i = 0; i < clip.getItemCount() && count < MAX_TRACKS; i++) {
                Uri uri = clip.getItemAt(i).getUri();
                if (uri != null) {
                    line(out, uri, "", displayName(uri));
                    count++;
                }
            }
        } else if (data.getData() != null) {
            line(out, data.getData(), "", displayName(data.getData()));
            count++;
        }
        return count;
    }

    private String displayName(Uri uri) {
        try (Cursor cursor = activity.getContentResolver().query(
                uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst() && !cursor.isNull(0)) {
                return cursor.getString(0);
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "no name for " + uri + ": " + e);
        }
        String last = uri.getLastPathSegment();
        return last == null ? "track" : last;
    }

    /** Every audio document below the tree, breadth first. */
    private int listTree(Uri tree, StringBuilder out) {
        if (tree == null) {
            return 0;
        }
        ContentResolver resolver = activity.getContentResolver();
        String[] columns = {
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
        };
        ArrayDeque<String[]> folders = new ArrayDeque<>();
        // (document id, the folder's path below the tree, depth)
        folders.add(new String[] {DocumentsContract.getTreeDocumentId(tree), "", "0"});
        int count = 0;
        while (!folders.isEmpty() && count < MAX_TRACKS) {
            String[] folder = folders.poll();
            int depth = Integer.parseInt(folder[2]);
            Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, folder[0]);
            try (Cursor cursor = resolver.query(children, columns, null, null, null)) {
                while (cursor != null && cursor.moveToNext() && count < MAX_TRACKS) {
                    String id = cursor.getString(0);
                    String name = cursor.isNull(1) ? "" : cursor.getString(1);
                    String mime = cursor.isNull(2) ? "" : cursor.getString(2);
                    if (DocumentsContract.Document.MIME_TYPE_DIR.equals(mime)) {
                        if (depth < MAX_DEPTH) {
                            String path = folder[1].isEmpty() ? name : folder[1] + "/" + name;
                            folders.add(new String[] {id, path, String.valueOf(depth + 1)});
                        }
                    } else if (isAudio(mime, name)) {
                        line(out, DocumentsContract.buildDocumentUriUsingTree(tree, id),
                                folder[1], name);
                        count++;
                    }
                }
            } catch (RuntimeException e) {
                Log.w(TAG, "cannot list " + children + ": " + e);
            }
        }
        return count;
    }

    /** By MIME type, and by the name's ending where a provider knows none. */
    private static boolean isAudio(String mime, String name) {
        if (mime.startsWith("audio/") || mime.equals("application/ogg")) {
            return true;
        }
        String lower = name.toLowerCase(Locale.ROOT);
        for (String ending : new String[] {
            ".mp3", ".flac", ".ogg", ".oga", ".opus", ".m4a", ".m4b", ".aac", ".wav", ".aiff",
            ".aif", ".caf", ".mka", ".weba",
        }) {
            if (lower.endsWith(ending)) {
                return true;
            }
        }
        return false;
    }

    // -- audio focus --------------------------------------------------------------------------

    private boolean requestFocus() {
        AudioManager manager = (AudioManager) activity.getSystemService(Context.AUDIO_SERVICE);
        if (manager == null) {
            return true;
        }
        if (focusRequest == null) {
            focusRequest = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                    .setAudioAttributes(new AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                            .build())
                    .setOnAudioFocusChangeListener(focusListener,
                            new Handler(Looper.getMainLooper()))
                    .build();
        }
        boolean granted = manager.requestAudioFocus(focusRequest)
                == AudioManager.AUDIOFOCUS_REQUEST_GRANTED;
        if (granted) {
            watchNoisy(true);
        }
        return granted;
    }

    private void abandonFocus() {
        AudioManager manager = (AudioManager) activity.getSystemService(Context.AUDIO_SERVICE);
        if (manager != null && focusRequest != null) {
            manager.abandonAudioFocusRequest(focusRequest);
        }
        watchNoisy(false);
    }

    /** Headphones pulled out: the music does not go on from the speaker. */
    private synchronized void watchNoisy(boolean on) {
        if (on == noisyRegistered) {
            return;
        }
        if (on) {
            IntentFilter filter = new IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY);
            if (Build.VERSION.SDK_INT >= 33) {
                activity.registerReceiver(noisyReceiver, filter, Context.RECEIVER_NOT_EXPORTED);
            } else {
                activity.registerReceiver(noisyReceiver, filter);
            }
        } else {
            activity.unregisterReceiver(noisyReceiver);
        }
        noisyRegistered = on;
    }

    void release() {
        try {
            abandonFocus();
        } catch (RuntimeException e) {
            Log.w(TAG, "release: " + e);
        }
    }
}
