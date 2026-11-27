package io.github.c0decave.axrossamp;

import android.app.NativeActivity;
import android.content.Intent;
import android.os.Bundle;
import android.os.Process;
import android.util.Log;

/**
 * The one activity: a NativeActivity whose window the Rust side draws into
 * (libaxrossamp.so, android_main). Everything Java has to do lives in
 * {@link Bridge}, so that the activity's base class can change (GameActivity,
 * step 2) without touching it.
 */
public final class MainActivity extends NativeActivity {
    static {
        // NativeActivity loads the library by itself, but outside the class
        // loader's list: without this the VM does not find Bridge's native
        // method in it.
        System.loadLibrary("axrossamp");
    }

    private Bridge bridge;

    /** An activity of this process has started android_main already. */
    private static boolean started;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // One android_main a process (winit makes one event loop a process):
        // a second activity in the same one - the system or a launcher
        // starting it again before the first one is destroyed (a new task
        // over the old, CLEAR_TASK) - would start a second android_main that
        // cannot draw. So this one is started afresh, in a new process, and
        // this process ends now, before any native code runs again.
        if (started) {
            Log.i("axrossamp", "a second activity in one process: started afresh in a new one");
            Intent again = new Intent(getIntent());
            again.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
            startActivity(again);
            Process.killProcess(Process.myPid());
            return;
        }
        started = true;
        // (before super.onCreate: that starts android_main, which may call
        // bridgeCall at once)
        bridge = new Bridge(this);
        bridge.edgeToEdge();
        super.onCreate(savedInstanceState);
        bridge.watchInsets();
        bridge.catchMediaKeys();
        bridge.reportLastExit();
        bridge.testTracks(getIntent());
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        bridge.testTracks(intent);
    }

    /** Called from Rust (any thread): see Bridge.call. */
    @SuppressWarnings("unused")
    public int bridgeCall(int what, String arg) {
        Bridge b = bridge;
        return b == null ? -1 : b.call(what, arg);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        bridge.onActivityResult(requestCode, resultCode, data);
    }

    @Override
    protected void onStart() {
        super.onStart();
        bridge.startWatchdog();
        Bridge.event(Bridge.EVENT_LIFECYCLE, 1, 0, 0, 0, null);
    }

    @Override
    protected void onStop() {
        // (the music plays on: the playback service keeps the process; the
        // watchdog does not wake a hidden app)
        Bridge.event(Bridge.EVENT_LIFECYCLE, 0, 0, 0, 0, null);
        bridge.stopWatchdog();
        super.onStop();
    }

    /**
     * The activity is gone for good (the task swiped away, a configuration
     * change the manifest does not list, "Don't keep activities"): so is the
     * process. android_main cannot run twice in one process (winit makes one
     * event loop a process), and the NativeActivity glue would wait here for
     * an android_main that never returns (winit 0.30 ignores Destroy) - the
     * main thread blocked for good, an ANR. The next start is a fresh
     * process.
     */
    @Override
    protected void onDestroy() {
        bridge.stopWatchdog();
        bridge.release();
        Log.i("axrossamp", "the activity is destroyed: the process ends");
        Process.killProcess(Process.myPid());
        super.onDestroy();
    }
}
