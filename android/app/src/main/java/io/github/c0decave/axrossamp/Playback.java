package io.github.c0decave.axrossamp;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.media.MediaMetadata;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;

/**
 * What plays, as the system shows it: one MediaSession for the process -
 * the lock screen, a headset's and Bluetooth's buttons, Android 13's media
 * controls in the quick settings - and the media notification of the
 * playback service ({@link PlaybackService}), which keeps the process while
 * music plays with the app hidden or the screen off.
 *
 * The Rust side tells it every change (CALL_PLAYBACK: state, position,
 * length, title, artist, album); every command - the session's, the
 * notification's - goes back as EVENT_MEDIA, to the player's own thread.
 * Framework classes only (android.media.session): no library in the APK.
 */
final class Playback {
    private static final String TAG = "axrossamp";
    static final String CHANNEL = "playback";
    static final int NOTIFICATION = 1;

    // PlaybackState's states, as the Rust side counts them
    static final int NONE = 0;
    static final int STOPPED = 1;
    static final int PAUSED = 2;
    static final int PLAYING = 3;

    private static Playback instance;

    static synchronized Playback get(Context context) {
        if (instance == null) {
            instance = new Playback(context.getApplicationContext());
        }
        return instance;
    }

    /** The process's playback, if there is one (a notification's button in a process just started has none). */
    static synchronized Playback peek() {
        return instance;
    }

    final Context context;
    final MediaSession session;
    private final Handler main = new Handler(Looper.getMainLooper());

    volatile int state = NONE;
    private long position;
    private long duration;
    private long updatedAt;
    private String title = "";
    private String artist = "";
    private String album = "";

    private Playback(Context context) {
        this.context = context;
        session = new MediaSession(context, "axross-amp");
        session.setCallback(new MediaSession.Callback() {
            @Override
            public void onPlay() {
                send(Bridge.MEDIA_PLAY, 0);
            }

            @Override
            public void onPause() {
                send(Bridge.MEDIA_PAUSE, 0);
            }

            @Override
            public void onStop() {
                send(Bridge.MEDIA_STOP, 0);
            }

            @Override
            public void onSkipToNext() {
                send(Bridge.MEDIA_NEXT, 0);
            }

            @Override
            public void onSkipToPrevious() {
                send(Bridge.MEDIA_PREVIOUS, 0);
            }

            @Override
            public void onFastForward() {
                send(Bridge.MEDIA_FORWARD, 0);
            }

            @Override
            public void onRewind() {
                send(Bridge.MEDIA_REWIND, 0);
            }

            @Override
            public void onSeekTo(long ms) {
                send(Bridge.MEDIA_SEEK_TO, (int) Math.max(0, Math.min(ms, Integer.MAX_VALUE)));
            }

            @Override
            public void onCustomAction(String action, android.os.Bundle extras) {
                if ("rewind".equals(action)) {
                    send(Bridge.MEDIA_REWIND, 0);
                } else if ("forward".equals(action)) {
                    send(Bridge.MEDIA_FORWARD, 0);
                }
            }
        }, main);
        session.setSessionActivity(openApp(context));
        if (Build.VERSION.SDK_INT >= 26) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL, "Playback", NotificationManager.IMPORTANCE_LOW);
            channel.setDescription("What plays, with its buttons");
            channel.setShowBadge(false);
            NotificationManager manager = context.getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.createNotificationChannel(channel);
            }
        }
    }

    static void send(int command, int arg) {
        Bridge.event(Bridge.EVENT_MEDIA, command, arg, 0, 0, null);
    }

    static PendingIntent openApp(Context context) {
        Intent intent = new Intent(context, MainActivity.class)
                .setAction(Intent.ACTION_MAIN)
                .addCategory(Intent.CATEGORY_LAUNCHER);
        return PendingIntent.getActivity(context, 0, intent,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    /** From the Rust side (any thread): "state TAB position TAB length TAB title TAB artist TAB album". */
    void update(String line) {
        String[] parts = (line == null ? "" : line).split("\t", -1);
        if (parts.length < 6) {
            Log.w(TAG, "a playback line of " + parts.length + " parts");
            return;
        }
        int newState;
        try {
            newState = Integer.parseInt(parts[0]);
            position = Long.parseLong(parts[1]);
            duration = Long.parseLong(parts[2]);
        } catch (NumberFormatException e) {
            Log.w(TAG, "a playback line not understood: " + e);
            return;
        }
        title = parts[3];
        artist = parts[4];
        album = parts[5];
        updatedAt = SystemClock.elapsedRealtime();
        state = newState;

        session.setMetadata(new MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, title)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
                .putString(MediaMetadata.METADATA_KEY_ALBUM, album)
                .putLong(MediaMetadata.METADATA_KEY_DURATION, duration)
                .build());
        int framework;
        switch (newState) {
            case PLAYING:
                framework = PlaybackState.STATE_PLAYING;
                break;
            case PAUSED:
                framework = PlaybackState.STATE_PAUSED;
                break;
            case STOPPED:
                framework = PlaybackState.STATE_STOPPED;
                break;
            default:
                framework = PlaybackState.STATE_NONE;
        }
        session.setPlaybackState(new PlaybackState.Builder()
                .setActions(PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE
                        | PlaybackState.ACTION_PLAY_PAUSE | PlaybackState.ACTION_STOP
                        | PlaybackState.ACTION_SKIP_TO_NEXT | PlaybackState.ACTION_SKIP_TO_PREVIOUS
                        | PlaybackState.ACTION_FAST_FORWARD | PlaybackState.ACTION_REWIND
                        | PlaybackState.ACTION_SEEK_TO)
                // (15 s back and on as buttons too: Android 13's media
                // controls show custom actions, not the rewind action)
                .addCustomAction(new PlaybackState.CustomAction.Builder(
                        "rewind", "15 seconds back", R.drawable.ic_media_rewind).build())
                .addCustomAction(new PlaybackState.CustomAction.Builder(
                        "forward", "15 seconds on", R.drawable.ic_media_forward).build())
                .setState(framework, position, newState == PLAYING ? 1f : 0f, updatedAt)
                .build());
        // (active while there is something to play: a headset's button
        // reaches it, also paused)
        session.setActive(newState != NONE);
        main.post(() -> PlaybackService.follow(context, newState));
    }

    /** The media notification for the current state. */
    Notification notification() {
        Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(context, CHANNEL)
                : new Notification.Builder(context);
        boolean playing = state == PLAYING;
        builder.setSmallIcon(R.drawable.ic_launcher_monochrome)
                .setContentTitle(title.isEmpty() ? "axross-amp" : title)
                .setContentText(artist.isEmpty() ? album : artist)
                .setContentIntent(session.getController().getSessionActivity())
                .setDeleteIntent(MediaActions.intent(context, MediaActions.DISMISS))
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                .setOnlyAlertOnce(true)
                .setShowWhen(false)
                .setOngoing(playing)
                .addAction(action(R.drawable.ic_media_previous, "Previous", Bridge.MEDIA_PREVIOUS))
                .addAction(action(R.drawable.ic_media_rewind, "15 seconds back", Bridge.MEDIA_REWIND))
                .addAction(playing
                        ? action(R.drawable.ic_media_pause, "Pause", Bridge.MEDIA_PAUSE)
                        : action(R.drawable.ic_media_play, "Play", Bridge.MEDIA_PLAY))
                .addAction(action(R.drawable.ic_media_forward, "15 seconds on", Bridge.MEDIA_FORWARD))
                .addAction(action(R.drawable.ic_media_next, "Next", Bridge.MEDIA_NEXT))
                .setStyle(new Notification.MediaStyle()
                        .setMediaSession(session.getSessionToken())
                        .setShowActionsInCompactView(0, 2, 4));
        return builder.build();
    }

    private Notification.Action action(int icon, String label, int command) {
        return new Notification.Action.Builder(
                android.graphics.drawable.Icon.createWithResource(context, icon), label,
                MediaActions.intent(context, command)).build();
    }
}
