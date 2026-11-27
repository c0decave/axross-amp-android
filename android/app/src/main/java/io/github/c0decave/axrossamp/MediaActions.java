package io.github.c0decave.axrossamp;

import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.util.Log;

/**
 * The media notification's buttons (and its dismissal): broadcasts to this
 * receiver of the app's own (not exported), which hands them to the player
 * as the media session's commands are. A broadcast works with the app in
 * the background, where starting a service would not.
 */
public final class MediaActions extends BroadcastReceiver {
    static final String ACTION = "io.github.c0decave.axrossamp.MEDIA";
    /** Not a media command: the notification was swiped away. */
    static final int DISMISS = -1;

    static PendingIntent intent(Context context, int command) {
        Intent intent = new Intent(context, MediaActions.class)
                .setAction(ACTION)
                .putExtra("command", command);
        // (one PendingIntent a command: the request code tells them apart)
        return PendingIntent.getBroadcast(context, 100 + command, intent,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    @Override
    public void onReceive(Context context, Intent intent) {
        int command = intent.getIntExtra("command", 0);
        Playback playback = Playback.peek();
        if (playback == null) {
            // (a notification left over from a process that is gone: no
            // player here to command)
            Log.i("axrossamp", "a media button with no player: the notification goes");
            NotificationManager manager = context.getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.cancel(Playback.NOTIFICATION);
            }
            return;
        }
        if (command == DISMISS) {
            PlaybackService.dismissed(context);
            return;
        }
        Playback.send(command, 0);
    }
}
