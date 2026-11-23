package io.github.c0decave.axrossamp;

import android.app.NativeActivity;
import android.content.Intent;
import android.os.Bundle;

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

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // (before super.onCreate: that starts android_main, which may call
        // bridgeCall at once)
        bridge = new Bridge(this);
        bridge.edgeToEdge();
        super.onCreate(savedInstanceState);
        bridge.watchInsets();
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
        Bridge.event(Bridge.EVENT_LIFECYCLE, 1, 0, 0, 0, null);
    }

    @Override
    protected void onStop() {
        // (no foreground service yet: the music does not go on unseen)
        Bridge.event(Bridge.EVENT_LIFECYCLE, 0, 0, 0, 0, null);
        super.onStop();
    }

    @Override
    protected void onDestroy() {
        bridge.release();
        super.onDestroy();
    }
}
