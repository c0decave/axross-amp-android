package io.github.c0decave.axrossamp;

import android.app.Notification;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

/**
 * The playback service: while music plays, a foreground service of the
 * type mediaPlayback with the media notification ({@link Playback}), so
 * that Android keeps the process - and the player's thread, the engine and
 * its sound - with the app hidden, the screen off, on the lock screen.
 *
 * It follows the player's state ({@link #follow}): playing - started (from
 * the user's play, or from the media session's or notification's command:
 * the cases Android 12+ lets an app start a foreground service in) and in
 * the foreground; paused - out of the foreground, its notification stays
 * and can be swiped away (which ends the service); stopped or nothing to
 * play - ended. The player itself lives in the process, not in this
 * service: the service is what keeps the process.
 *
 * Android 9 needs FOREGROUND_SERVICE; 10+ the type in the manifest; 14+
 * FOREGROUND_SERVICE_MEDIA_PLAYBACK and the type at startForeground.
 */
public final class PlaybackService extends Service {
    private static final String TAG = "axrossamp";

    /** The running service (main thread only). */
    private static PlaybackService running;

    /** On the main thread: the service as the player's state wants it. */
    static void follow(Context context, int state) {
        if (state == Playback.PLAYING) {
            if (running == null) {
                Intent intent = new Intent(context, PlaybackService.class);
                try {
                    if (Build.VERSION.SDK_INT >= 26) {
                        context.startForegroundService(intent);
                    } else {
                        context.startService(intent);
                    }
                } catch (RuntimeException e) {
                    // (Android 12+ refuses a start from the background
                    // outside the allowed cases: the music plays on, the
                    // process is not kept for it)
                    Log.w(TAG, "the playback service could not start: " + e);
                }
            } else {
                running.show(state);
            }
        } else if (running != null) {
            if (state == Playback.PAUSED) {
                running.show(state);
            } else {
                running.end();
            }
        }
    }

    /** The notification was swiped away (it can be while paused). */
    static void dismissed(Context context) {
        Playback playback = Playback.peek();
        if (running != null && (playback == null || playback.state != Playback.PLAYING)) {
            running.end();
        }
    }

    @Override
    public void onCreate() {
        super.onCreate();
        running = this;
        Log.i(TAG, "the playback service is there");
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        Playback playback = Playback.peek();
        // (started with startForegroundService: in the foreground at once,
        // whatever the state is by now - an Android 8+ rule - then as the
        // state wants it)
        show(Playback.PLAYING);
        int state = playback == null ? Playback.NONE : playback.state;
        if (state == Playback.PAUSED) {
            show(state);
        } else if (state != Playback.PLAYING) {
            end();
        }
        // (if the process is gone, the service is not to come back alone:
        // the player that played is gone with it)
        return START_NOT_STICKY;
    }

    private void show(int state) {
        Playback playback = Playback.peek();
        Notification notification = playback != null
                ? playback.notification()
                : new Notification.Builder(this, Playback.CHANNEL)
                        .setSmallIcon(R.drawable.ic_launcher_monochrome)
                        .setContentTitle("axross-amp")
                        .build();
        if (state == Playback.PLAYING) {
            try {
                if (Build.VERSION.SDK_INT >= 29) {
                    startForeground(Playback.NOTIFICATION, notification,
                            ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
                } else {
                    startForeground(Playback.NOTIFICATION, notification);
                }
            } catch (RuntimeException e) {
                // (back to the foreground from the background, refused on
                // Android 12+ outside the allowed cases: the notification
                // at least)
                Log.w(TAG, "the playback service could not come to the foreground: " + e);
                notifyPlain(notification);
            }
        } else {
            // (paused: out of the foreground, the notification stays and
            // may be swiped away)
            if (Build.VERSION.SDK_INT >= 24) {
                stopForeground(STOP_FOREGROUND_DETACH);
            } else {
                stopForeground(false);
            }
            notifyPlain(notification);
        }
    }

    private void notifyPlain(Notification notification) {
        NotificationManager manager = getSystemService(NotificationManager.class);
        if (manager != null) {
            try {
                manager.notify(Playback.NOTIFICATION, notification);
            } catch (RuntimeException e) {
                // (Android 13+ without the notification permission)
                Log.w(TAG, "no notification: " + e);
            }
        }
    }

    private void end() {
        if (Build.VERSION.SDK_INT >= 24) {
            stopForeground(STOP_FOREGROUND_REMOVE);
        } else {
            stopForeground(true);
        }
        NotificationManager manager = getSystemService(NotificationManager.class);
        if (manager != null) {
            manager.cancel(Playback.NOTIFICATION);
        }
        stopSelf();
    }

    @Override
    public void onDestroy() {
        if (running == this) {
            running = null;
        }
        Log.i(TAG, "the playback service ends");
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
