# The names the Rust side calls by name over JNI: kept, should minify ever
# be turned on (it is off: two small classes, nothing to shrink). Without
# them every call from Rust would fail (-1) and no event would arrive.
-keep class io.github.c0decave.axrossamp.MainActivity {
    public int bridgeCall(int, java.lang.String);
}
-keepclasseswithmembernames class io.github.c0decave.axrossamp.Bridge {
    native <methods>;
}
