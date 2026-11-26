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
import android.os.CancellationSignal;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.os.SystemClock;
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
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;

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
    /** The main thread has been blocked a = that many ms (the watchdog). */
    static final int EVENT_STALL = 7;

    private static final int REQUEST_FOLDER = 4201;
    private static final int REQUEST_FILES = 4202;

    /** A folder's tracks: no more than this many, no deeper than this. */
    private static final int MAX_TRACKS = 5000;
    private static final int MAX_DEPTH = 12;

    /**
     * A folder's tracks go to the Rust side in batches while it is read, so
     * that the first plays at once: the first batch at this many tracks (or
     * sooner, see BATCH_MS), the others at BATCH_MORE.
     */
    private static final int BATCH_FIRST = 25;
    private static final int BATCH_MORE = 250;
    private static final long BATCH_MS = 300;
    /** A folder whose listing takes longer than this is logged. */
    private static final long SLOW_QUERY_MS = 1000;
    /** The main thread blocked longer than this is logged (the watchdog). */
    private static final long STALL_MS = 2000;
    /**
     * A provider that has not handed a document over by then is asked to
     * stop (the Rust side gives up at 12 s; this ends the binder call
     * where the provider honours it, so the waiting thread goes too).
     */
    private static final long OPEN_CANCEL_MS = 11000;

    /** Counts the picks: a newer one ends the reading of an older one. */
    private final AtomicInteger picks = new AtomicInteger();
    private final boolean debuggable;

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
        this.debuggable = (activity.getApplicationInfo().flags
                & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0;
    }

    /** A file's name for the log: only a debug build logs names. */
    private String logName(String name) {
        if (debuggable) {
            return name;
        }
        int dot = name == null ? -1 : name.lastIndexOf('.');
        return dot < 0 ? "a file" : "a " + name.substring(dot) + " file";
    }

    /**
     * Debug builds only: files handed over by the emulator's test (`am start
     * ... --esa amp.paths /sdcard/...,...`) are played as if picked - no
     * picker can be driven there. A release ignores the extra.
     */
    void testTracks(Intent intent) {
        if (!debuggable || intent == null) {
            return;
        }
        String[] paths = intent.getStringArrayExtra("amp.paths");
        if (paths == null || paths.length == 0) {
            return;
        }
        StringBuilder out = new StringBuilder();
        for (String path : paths) {
            int slash = path.lastIndexOf('/');
            line(out, path, "", slash < 0 ? path : path.substring(slash + 1), -1);
        }
        Log.i(TAG, "test tracks: " + paths.length);
        event(EVENT_PICKED, paths.length, 0, picks.incrementAndGet(), paths.length, out.toString());
    }

    // -- the watchdog -------------------------------------------------------------------------

    private volatile long beat = SystemClock.uptimeMillis();
    /** Counts the watchdog's starts and stops: a thread runs while it is its own. */
    private final AtomicInteger watchdog = new AtomicInteger();

    /**
     * Twice a second the main thread is asked to beat; when it has not for
     * STALL_MS, what it and the app's threads are doing goes to the log -
     * Java's stacks here, the Rust side's steps through EVENT_STALL - once
     * a stall, and its end when it ends.
     */
    void startWatchdog() {
        int mine = watchdog.incrementAndGet();
        beat = SystemClock.uptimeMillis();
        Handler main = new Handler(Looper.getMainLooper());
        Thread dog = new Thread(() -> {
            boolean told = false;
            while (watchdog.get() == mine) {
                main.post(() -> beat = SystemClock.uptimeMillis());
                try {
                    Thread.sleep(500);
                } catch (InterruptedException e) {
                    return;
                }
                long blocked = SystemClock.uptimeMillis() - beat;
                if (blocked > STALL_MS && !told) {
                    told = true;
                    logStall(blocked);
                } else if (blocked <= 600 && told) {
                    told = false;
                    Log.w(TAG, "the main thread goes on again");
                }
            }
        }, "amp-watchdog");
        dog.setDaemon(true);
        dog.start();
    }

    /** Stopped while the activity is hidden: nothing wakes a hidden app. */
    void stopWatchdog() {
        watchdog.incrementAndGet();
    }

    private void logStall(long blocked) {
        Log.w(TAG, "the main thread has been blocked for " + blocked + " ms; its stack:");
        Thread main = Looper.getMainLooper().getThread();
        logStack(main, main.getStackTrace());
        for (Map.Entry<Thread, StackTraceElement[]> entry : Thread.getAllStackTraces().entrySet()) {
            Thread thread = entry.getKey();
            if (thread != main && thread.getName().startsWith("amp")) {
                logStack(thread, entry.getValue());
            }
        }
        event(EVENT_STALL, (int) Math.min(blocked, Integer.MAX_VALUE), 0, 0, 0, null);
    }

    private static void logStack(Thread thread, StackTraceElement[] stack) {
        StringBuilder text = new StringBuilder();
        text.append(thread.getName()).append(" (").append(thread.getState()).append(')');
        for (int i = 0; i < stack.length && i < 24; i++) {
            text.append("\n    at ").append(stack[i]);
        }
        Log.w(TAG, text.toString());
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
            event(EVENT_PICKED, -1, 0, picks.incrementAndGet(), 0, "");
        }
    }

    private int openFd(String uri) {
        if (uri == null) {
            return -1;
        }
        long asked = SystemClock.uptimeMillis();
        CancellationSignal signal = new CancellationSignal();
        Handler main = new Handler(Looper.getMainLooper());
        Runnable cancel = signal::cancel;
        main.postDelayed(cancel, OPEN_CANCEL_MS);
        try {
            ParcelFileDescriptor pfd =
                    activity.getContentResolver().openFileDescriptor(Uri.parse(uri), "r", signal);
            long took = SystemClock.uptimeMillis() - asked;
            if (took > SLOW_QUERY_MS) {
                Log.w(TAG, "the provider took " + took + " ms to open a document");
            }
            return pfd == null ? -1 : pfd.detachFd();
        } catch (Exception e) {
            Log.w(TAG, "cannot open " + (debuggable ? uri : "a document") + ": " + e);
            return -1;
        } finally {
            main.removeCallbacks(cancel);
        }
    }

    // -- the pickers' answers -----------------------------------------------------------------

    void onActivityResult(int request, int result, Intent data) {
        if (request != REQUEST_FOLDER && request != REQUEST_FILES) {
            return;
        }
        int pick = picks.incrementAndGet();
        if (result != Activity.RESULT_OK || data == null) {
            // (nothing chosen: an empty last batch)
            event(EVENT_PICKED, 0, 0, pick, 0, "");
            return;
        }
        // (a folder is read off the main thread: it may hold thousands, and
        // a busy provider answers slowly)
        Thread reader = new Thread(() -> {
            Batches batches = new Batches(pick);
            long started = SystemClock.uptimeMillis();
            Log.i(TAG, "pick " + pick + ": reading " + (request == REQUEST_FOLDER ? "a folder" : "the files"));
            try {
                if (request == REQUEST_FOLDER) {
                    listTree(data.getData(), batches);
                } else {
                    listFiles(data, batches);
                }
            } catch (RuntimeException e) {
                Log.e(TAG, "reading the choice failed", e);
            }
            batches.last();
            Log.i(TAG, "pick " + pick + ": " + batches.total + " tracks in "
                    + (SystemClock.uptimeMillis() - started) + " ms"
                    + (batches.current() ? "" : " (a newer pick came)"));
        }, "amp-picked");
        reader.start();
    }

    /**
     * The tracks of one pick, to the Rust side in batches: EVENT_PICKED with
     * a = the batch's tracks, b = 1 while more follow (0: the last), c = the
     * pick, d = the tracks so far.
     */
    private final class Batches {
        final int pick;
        final StringBuilder out = new StringBuilder();
        int inBatch;
        int total;
        int sent;
        long since = SystemClock.uptimeMillis();

        Batches(int pick) {
            this.pick = pick;
        }

        /** Whether this pick is still the latest. */
        boolean current() {
            return picks.get() == pick;
        }

        void add(Uri uri, String folder, String name, long size) {
            line(out, uri.toString(), folder, name, size);
            inBatch++;
            total++;
            int size = sent == 0 ? BATCH_FIRST : BATCH_MORE;
            if (inBatch >= size || SystemClock.uptimeMillis() - since >= BATCH_MS) {
                send(true);
            }
        }

        void send(boolean more) {
            if (!current()) {
                return;
            }
            event(EVENT_PICKED, inBatch, more ? 1 : 0, pick, total, out.toString());
            Log.i(TAG, "pick " + pick + ": " + total + " tracks so far" + (more ? "" : ", done"));
            out.setLength(0);
            inBatch = 0;
            sent++;
            since = SystemClock.uptimeMillis();
        }

        void last() {
            send(false);
        }
    }

    /**
     * One line a track: uri, tab, folder ("" at the top), tab, name, tab,
     * the size the provider gives (-1: none).
     */
    private static void line(StringBuilder out, String uri, String folder, String name, long size) {
        out.append(clean(uri)).append('\t').append(clean(folder)).append('\t').append(clean(name))
                .append('\t').append(size).append('\n');
    }

    private static String clean(String text) {
        return text == null ? "" : text.replace('\t', ' ').replace('\n', ' ').replace('\r', ' ');
    }

    private void listFiles(Intent data, Batches batches) {
        ClipData clip = data.getClipData();
        if (clip != null) {
            for (int i = 0; i < clip.getItemCount() && batches.total < MAX_TRACKS; i++) {
                Uri uri = clip.getItemAt(i).getUri();
                if (uri != null && batches.current()) {
                    batches.add(uri, "", displayName(uri), -1);
                }
            }
        } else if (data.getData() != null) {
            batches.add(data.getData(), "", displayName(data.getData()), -1);
        }
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
    private void listTree(Uri tree, Batches batches) {
        if (tree == null) {
            return;
        }
        ContentResolver resolver = activity.getContentResolver();
        String[] columns = {
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE,
        };
        ArrayDeque<String[]> folders = new ArrayDeque<>();
        // (document id, the folder's path below the tree, depth)
        folders.add(new String[] {DocumentsContract.getTreeDocumentId(tree), "", "0"});
        while (!folders.isEmpty() && batches.total < MAX_TRACKS && batches.current()) {
            String[] folder = folders.poll();
            int depth = Integer.parseInt(folder[2]);
            Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, folder[0]);
            long asked = SystemClock.uptimeMillis();
            try (Cursor cursor = resolver.query(children, columns, null, null, null)) {
                long took = SystemClock.uptimeMillis() - asked;
                if (took > SLOW_QUERY_MS) {
                    Log.w(TAG, "a folder's listing took " + took + " ms (a busy provider)");
                }
                while (cursor != null && cursor.moveToNext() && batches.total < MAX_TRACKS) {
                    if (!batches.current()) {
                        return;
                    }
                    String id = cursor.getString(0);
                    String name = cursor.isNull(1) ? "" : cursor.getString(1);
                    String mime = cursor.isNull(2) ? "" : cursor.getString(2);
                    if (DocumentsContract.Document.MIME_TYPE_DIR.equals(mime)) {
                        if (depth < MAX_DEPTH) {
                            String path = folder[1].isEmpty() ? name : folder[1] + "/" + name;
                            folders.add(new String[] {id, path, String.valueOf(depth + 1)});
                        }
                    } else if (isAudio(mime, name)) {
                        long size = cursor.isNull(3) ? -1 : cursor.getLong(3);
                        batches.add(DocumentsContract.buildDocumentUriUsingTree(tree, id),
                                folder[1], name, size);
                    }
                }
            } catch (RuntimeException e) {
                Log.w(TAG, "cannot list a folder (" + logName(folder[1]) + "): " + e);
            }
        }
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
