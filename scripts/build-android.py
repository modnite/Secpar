#!/usr/bin/env python3
"""Build the Secpar Android app.

The app is the pinned Moonlight Android source (upstream/pins.json) with Secpar's
screen in front of it: android-secpar/ holds the added files, and the changes to
Moonlight's own build file, manifest and stream screen are made here, each checked
so a new upstream version cannot be patched wrongly without notice.

Usage: build-android.py <moonlight-android source> <work folder> <folder with libsecpar_android.so> [gradle task]
Needs JAVA_HOME and ANDROID_HOME. The library comes from scripts/build-android-core.sh.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

NDK = "28.2.13676358"
APPLICATION_ID = "win.secpar.app"
# The version comes from version.txt, and the Android version code from the same
# numbers: 4.0.0 becomes 40000. Android refuses to install a build whose version
# code is not higher than the installed one, so it must follow the release, not
# Moonlight's own counter.
VERSION = (Path(__file__).resolve().parent.parent / "version.txt").read_text(encoding="utf-8").strip()
if not all(part.isdigit() for part in VERSION.split(".")) or len(VERSION.split(".")) != 3:
    sys.exit(f"version.txt must be X.Y.Z, found {VERSION!r}")
VERSION_CODE = int(VERSION.split(".")[0]) * 10000 + int(VERSION.split(".")[1]) * 100 + int(VERSION.split(".")[2])


def edit(path: Path, pairs):
    text = path.read_bytes().decode("utf-8")
    for old, new in pairs:
        if text.count(old) != 1:
            sys.exit(f"{path.name}: expected exactly one {old!r}")
        text = text.replace(old, new)
    path.write_bytes(text.encode("utf-8"))


def check_overlay_in_apk(work: Path):
    """Fails the build when a release APK no longer carries Secpar's overlay.

    The class is built by Moonlight's stream activity, so a shrinker change that
    drops or renames it would leave a stream with no button and no error.
    """
    import zipfile

    marker = b"Lwin/secpar/app/SecparStreamOverlay;"
    apks = sorted((work / "app" / "build" / "outputs" / "apk").rglob("*.apk"))
    for apk in apks:
        with zipfile.ZipFile(apk) as archive:
            dexes = [name for name in archive.namelist() if name.endswith(".dex")]
            if not any(marker in archive.read(name) for name in dexes):
                sys.exit(f"{apk.name}: SecparStreamOverlay is missing from the dex")


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    source, work, library = (Path(p).resolve() for p in sys.argv[1:4])
    task = sys.argv[4] if len(sys.argv) > 4 else "assembleNonRootDebug"
    here = Path(__file__).resolve().parent.parent
    overlay = here / "android-secpar"

    if work.exists():
        shutil.rmtree(work)
    shutil.copytree(source, work, ignore=shutil.ignore_patterns(".git", "build", ".gradle", ".cxx"))
    main_dir = work / "app" / "src" / "main"
    shutil.copytree(overlay / "java", main_dir / "java", dirs_exist_ok=True)
    jni = main_dir / "jniLibs" / "arm64-v8a"
    jni.mkdir(parents=True, exist_ok=True)
    shutil.copy2(library / "libsecpar_android.so", jni)
    # Secpar's launcher icon, same file names as Moonlight's.
    for folder in (here / "app" / "src" / "main" / "res").glob("mipmap-*dpi"):
        for name in ("ic_launcher.png", "ic_launcher_foreground.png"):
            if (folder / name).is_file() and (main_dir / "res" / folder.name).is_dir():
                shutil.copy2(folder / name, main_dir / "res" / folder.name / name)

    # Secpar's own resources: the approved triangle and wordmark, and the Lucide
    # icons its screens use. Same checked copy as the Java overlay above.
    for item in (overlay / "res").iterdir():
        if item.is_dir():
            shutil.copytree(item, main_dir / "res" / item.name, dirs_exist_ok=True)
    # Secpar's own checks for the sizing rules, run by the app's unit test task.
    tests = overlay / "test" / "java"
    if tests.is_dir():
        shutil.copytree(tests, work / "app" / "src" / "test" / "java", dirs_exist_ok=True)

    # The icon sits on black.
    shutil.copy2(here / "app" / "src" / "main" / "res" / "values" / "ic_launcher_background.xml", main_dir / "res" / "values" / "ic_launcher_background.xml")

    edit(work / "app" / "build.gradle", [
        ('ndkVersion "29.0.14206865"', f'ndkVersion "{NDK}"'),
        ('        versionName "12.2"\n', f'        versionName "{VERSION}"\n'),
        ('        versionCode = 315\n', f'        versionCode = {VERSION_CODE}\n'),
        ("dependencies {\n", "dependencies {\n    testImplementation 'junit:junit:4.13.2'\n    implementation 'androidx.annotation:annotation:1.9.0'\n"),
        ("        minSdk 21\n", "        minSdk 26\n        ndk { abiFilters 'arm64-v8a' }\n"),
        ('            applicationId "com.limelight"\n', f'            applicationId "{APPLICATION_ID}"\n'),
        ('            resValue "string", "app_label", "Moonlight (Debug)"', '            resValue "string", "app_label", "Secpar 4 (test)"'),
        ('            applicationIdSuffix ".unofficial"\n', ""),
        ('            resValue "string", "app_label", "Moonlight"', '            resValue "string", "app_label", "Secpar"'),
    ])
    launcher = '''            <intent-filter>
                <action android:name="android.intent.action.MAIN" />

                <category android:name="android.intent.category.LAUNCHER" />
                <category android:name="android.intent.category.MULTIWINDOW_LAUNCHER" />
                <category android:name="android.intent.category.LEANBACK_LAUNCHER" />
                <category android:name="tv.ouya.intent.category.APP" />
            </intent-filter>
        </activity>
'''
    edit(main_dir / "AndroidManifest.xml", [
        # Secpar's screen is what opens; Moonlight's own computer list is not offered.
        (launcher, '''        </activity>

        <activity
            android:name="win.secpar.app.SecparActivity"
            android:exported="true"
            android:resizeableActivity="true"
            android:windowSoftInputMode="adjustResize"
            android:configChanges="keyboard|keyboardHidden|navigation|screenLayout|orientation|screenSize|smallestScreenSize|uiMode">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
'''),
    ])
    # Debug builds only: the review emulator has no account and no paired
    # computer, so the stream screen has to be started from adb. Exporting it
    # there lets `am start` reach Moonlight's own stream activity with the
    # Secpar extra, exactly as the app starts it. Release builds keep the
    # activity private.
    if "debug" in task.lower():
        edit(main_dir / "AndroidManifest.xml", [
            ('        <activity\n            android:name=".Game"\n',
             '        <activity\n            android:name=".Game"\n            android:exported="true"\n'),
        ])

    # Secpar's own connection states. The app has already opened the tunnel and
    # paired, and its computers screen shows "Starting" or "Connecting" on the
    # computer's card, so the stream screen is launched with one extra that keeps
    # Moonlight's progress, failure and pairing dialogs and its toasts out of a
    # Secpar flow and reports the state to Secpar instead. Every piece is checked
    # below, so a new upstream version fails here rather than silently reverting
    # to Moonlight's dialogs.
    edit(main_dir / "java" / "com" / "limelight" / "Game.java", [
        ('    public static final String EXTRA_SERVER_CERT = "ServerCert";\n', '''    public static final String EXTRA_SERVER_CERT = "ServerCert";
    // Secpar: the app that opens this stream shows its own connecting and error
    // states. With this extra set, this screen draws no progress or error dialog
    // and no toast, and reports what it is doing on this broadcast instead.
    public static final String EXTRA_SECPAR = "Secpar";
    public static final String SECPAR_EVENT = "win.secpar.app.STREAM";
    public static final String SECPAR_STATE = "state";
    public static final String SECPAR_REASON = "reason";
    public static final String SECPAR_CODE = "code";
'''),
        ('    private MediaCodecDecoderRenderer decoderRenderer;\n    private boolean reportedCrash;\n',
         '    private MediaCodecDecoderRenderer decoderRenderer;\n    private boolean reportedCrash;\n    // Secpar: true when the app asked for its own connection states.\n    private boolean secparMode;\n'),
        ('''        // Start the spinner
        spinner = SpinnerDialog.displayDialog(this, getResources().getString(R.string.conn_establishing_title),
                getResources().getString(R.string.conn_establishing_msg), true);
''',
         '''        // Start the spinner
        // Secpar: the app draws the progress on its own screen, so this screen
        // stays plain and no Moonlight progress dialog is created.
        secparMode = getIntent().getBooleanExtra(EXTRA_SECPAR, false);
        if (!secparMode) {
            spinner = SpinnerDialog.displayDialog(this, getResources().getString(R.string.conn_establishing_title),
                    getResources().getString(R.string.conn_establishing_msg), true);
        }
'''),
        ('''        if (connMgr.isActiveNetworkMetered()) {
            displayTransientMessage(getResources().getString(R.string.conn_metered));
        }
''',
         '''        if (!secparMode && connMgr.isActiveNetworkMetered()) {
            displayTransientMessage(getResources().getString(R.string.conn_metered));
        }
'''),
        ('''            // If we can't find an AVC decoder, we can't proceed
            Dialog.displayDialog(this, getResources().getString(R.string.conn_error_title),
                    "This device or ROM doesn't support hardware accelerated H.264 playback.", true);
            return;
''',
         '''            // If we can't find an AVC decoder, we can't proceed
            // Secpar: report it to the app instead of drawing a Moonlight dialog.
            if (secparMode) {
                reportToSecpar("failed", "no_decoder", 0);
            }
            else {
                Dialog.displayDialog(this, getResources().getString(R.string.conn_error_title),
                        "This device or ROM doesn't support hardware accelerated H.264 playback.", true);
            }
            return;
'''),
        ('''    @Override
    public void stageComplete(String stage) {
    }
''',
         '''    @Override
    public void stageComplete(String stage) {
    }

    /** Secpar: tells the app that asked for the stream what is happening to it. */
    private void reportToSecpar(String state, String reason, int code) {
        if (!secparMode) {
            return;
        }
        Intent update = new Intent(SECPAR_EVENT);
        update.setPackage(getPackageName());
        update.putExtra(SECPAR_STATE, state);
        update.putExtra(SECPAR_REASON, reason);
        update.putExtra(SECPAR_CODE, code);
        sendBroadcast(update);
    }
'''),
        ('''        if (conn != null) {
            int videoFormat = decoderRenderer.getActiveVideoFormat();
''',
         '''        if (conn != null) {
            // Secpar: leaving this screen while it is still connecting cancels the
            // attempt, so the tunnel and the computer are not kept waiting.
            if (secparMode && !connected) {
                displayedFailureDialog = true;
                new Thread() {
                    public void run() {
                        conn.stop();
                    }
                }.start();
                reportToSecpar("ended", "cancelled", 0);
            }

            int videoFormat = decoderRenderer.getActiveVideoFormat();
'''),
        ('''                if (!displayedFailureDialog) {
                    displayedFailureDialog = true;
                    LimeLog.severe(stage + " failed: " + errorCode);

                    // If video initialization failed and the surface is still valid, display extra information for the user
                    if (stage.contains("video") && streamView.getHolder().getSurface().isValid()) {
                        Toast.makeText(Game.this, getResources().getText(R.string.video_decoder_init_failed), Toast.LENGTH_LONG).show();
                    }

                    String dialogText = getResources().getString(R.string.conn_error_msg) + " " + stage +" (error "+errorCode+")";

                    if (portFlags != 0) {
                        dialogText += "\\n\\n" + getResources().getString(R.string.check_ports_msg) + "\\n" +
                                MoonBridge.stringifyPortFlags(portFlags, "\\n");
                    }

                    if (portTestResult != MoonBridge.ML_TEST_RESULT_INCONCLUSIVE && portTestResult != 0)  {
                        dialogText += "\\n\\n" + getResources().getString(R.string.nettest_text_blocked);
                    }

                    Dialog.displayDialog(Game.this, getResources().getString(R.string.conn_error_title), dialogText, true);
                }
''',
         '''                if (!displayedFailureDialog) {
                    displayedFailureDialog = true;
                    LimeLog.severe(stage + " failed: " + errorCode);

                    // Secpar: the app writes the sentence. This screen only says
                    // which part failed and closes, so no Moonlight dialog appears.
                    if (secparMode) {
                        reportToSecpar("failed", stage, errorCode);
                        finish();
                        return;
                    }

                    // If video initialization failed and the surface is still valid, display extra information for the user
                    if (stage.contains("video") && streamView.getHolder().getSurface().isValid()) {
                        Toast.makeText(Game.this, getResources().getText(R.string.video_decoder_init_failed), Toast.LENGTH_LONG).show();
                    }

                    String dialogText = getResources().getString(R.string.conn_error_msg) + " " + stage +" (error "+errorCode+")";

                    if (portFlags != 0) {
                        dialogText += "\\n\\n" + getResources().getString(R.string.check_ports_msg) + "\\n" +
                                MoonBridge.stringifyPortFlags(portFlags, "\\n");
                    }

                    if (portTestResult != MoonBridge.ML_TEST_RESULT_INCONCLUSIVE && portTestResult != 0)  {
                        dialogText += "\\n\\n" + getResources().getString(R.string.nettest_text_blocked);
                    }

                    Dialog.displayDialog(Game.this, getResources().getString(R.string.conn_error_title), dialogText, true);
                }
'''),
        ('''                    LimeLog.severe("Connection terminated: " + errorCode);
                    stopConnection();
''',
         '''                    LimeLog.severe("Connection terminated: " + errorCode);
                    stopConnection();

                    // Secpar: the app shows the ending in its own words and the
                    // stream screen closes quietly, with no dialog or toast.
                    if (secparMode) {
                        reportToSecpar(errorCode == MoonBridge.ML_ERROR_GRACEFUL_TERMINATION ? "ended" : "failed",
                                "connection", errorCode);
                        finish();
                        return;
                    }
'''),
        ('''                connected = true;
                connecting = false;
                updatePipAutoEnter();
''',
         '''                connected = true;
                connecting = false;
                updatePipAutoEnter();

                // Secpar: the picture is up, so its card can stop saying Connecting.
                reportToSecpar("connected", "", 0);
'''),
        ('''            // Update GameManager state to indicate we're "loading" while connecting
            UiHelper.notifyStreamConnecting(Game.this);
''',
         '''            // Update GameManager state to indicate we're "loading" while connecting
            UiHelper.notifyStreamConnecting(Game.this);

            // Secpar: the attempt to reach the computer has started.
            reportToSecpar("connecting", "", 0);
'''),
        ('''    public void displayMessage(final String message) {
        runOnUiThread(new Runnable() {
''',
         '''    public void displayMessage(final String message) {
        if (secparMode) {
            return;
        }
        runOnUiThread(new Runnable() {
'''),
        ('''    public void displayTransientMessage(final String message) {
        if (!prefConfig.disableWarnings) {
''',
         '''    public void displayTransientMessage(final String message) {
        if (secparMode) {
            return;
        }
        if (!prefConfig.disableWarnings) {
'''),
    ])

    # Secpar's button and panel over the running picture. Game.java is
    # Moonlight's own stream activity and MediaCodecDecoderRenderer is its
    # decoder, so every change is anchored on text that must appear exactly
    # once. A new upstream version fails here instead of building a half
    # patched stream screen.
    edit(main_dir / "java" / "com" / "limelight" / "Game.java", [
        ("import com.limelight.utils.UiHelper;\n",
         "import com.limelight.utils.UiHelper;\nimport win.secpar.app.SecparStreamOverlay;\n"),
        ("    private TextView performanceOverlayView;\n",
         "    private TextView performanceOverlayView;\n    private SecparStreamOverlay secparOverlay;\n"),
        ('''        performanceOverlayView = findViewById(R.id.performanceOverlay);

        inputCaptureProvider = InputCaptureManager.getInputCaptureProvider(this, this);
''',
         '''        performanceOverlayView = findViewById(R.id.performanceOverlay);

        // Secpar's button and panel over the picture. The panel asks the
        // decoder for the size and frame rate it is using, so a size change
        // during a stream shows up there.
        secparOverlay = new SecparStreamOverlay(this, (FrameLayout) streamView.getParent(),
                new SecparStreamOverlay.Host() {
            @Override
            public String computerName() {
                return pcName;
            }

            @Override
            public String pictureLine() {
                if (decoderRenderer == null || decoderRenderer.getActiveWidth() <= 0) {
                    return "Waiting for the picture";
                }
                return decoderRenderer.getActiveWidth() + " \\u00d7 "
                        + decoderRenderer.getActiveHeight() + " \\u00b7 "
                        + decoderRenderer.getActiveFrameRate() + " fps";
            }

            @Override
            public boolean statsVisible() {
                return performanceOverlayView.getVisibility() == View.VISIBLE;
            }

            @Override
            public void setStatsVisible(boolean visible) {
                // Moonlight's own performance overlay, for this stream.
                prefConfig.enablePerfOverlay = visible;
                performanceOverlayView.setVisibility(visible ? View.VISIBLE : View.GONE);
            }

            @Override
            public void disconnect() {
                // The same way out as Moonlight's quit key: onStop ends the
                // connection and returns to the Secpar app.
                finish();
            }
        });

        inputCaptureProvider = InputCaptureManager.getInputCaptureProvider(this, this);
'''),
        ('''    @Override
    protected void onDestroy() {
''',
         '''    @Override
    public void onBackPressed() {
        // Back closes Secpar's panel first, then leaves the stream.
        if (secparOverlay != null && secparOverlay.isOpen()) {
            secparOverlay.close();
            return;
        }
        super.onBackPressed();
    }

    @Override
    protected void onDestroy() {
'''),
    ])
    edit(main_dir / "java" / "com" / "limelight" / "binding" / "video" / "MediaCodecDecoderRenderer.java", [
        ('''    public int getActiveVideoFormat() {
        return this.videoFormat;
    }
''',
         '''    public int getActiveVideoFormat() {
        return this.videoFormat;
    }

    // The size and frame rate the decoder was set up with, for Secpar's
    // stream panel. A size change during a stream sets these again.
    public int getActiveWidth() {
        return this.initialWidth;
    }

    public int getActiveHeight() {
        return this.initialHeight;
    }

    public int getActiveFrameRate() {
        return this.refreshRate;
    }
'''),
    ])
    # Secpar's overlay must survive the release shrinker. The stream activity
    # builds these classes directly, so losing them is a silent loss of the
    # feature; the check below then fails the build if they are gone.
    proguard = work / "app" / "proguard-rules.pro"
    marker = "# Secpar: the in-stream button and panel"
    rules = proguard.read_bytes().decode("utf-8")
    if marker in rules:
        sys.exit("proguard-rules.pro already carries the Secpar overlay rules")
    proguard.write_bytes((rules.rstrip("\n") + "\n\n" + '''# Secpar: the in-stream button and panel. Kept by name so the stream screen's
# overrides and the dex check in scripts/build-android.py stay readable.
-keep class win.secpar.app.SecparStreamOverlay { *; }
-keep class win.secpar.app.SecparStreamOverlay$* { *; }
-keep interface win.secpar.app.SecparStreamOverlay$Host { *; }
-keep class win.secpar.app.Ui { *; }
''' + "\n").encode("utf-8"))

    gradle = work / ("gradlew.bat" if os.name == "nt" else "gradlew")
    result = subprocess.call([str(gradle), task, "--console=plain"], cwd=work)
    if result == 0:
        check_overlay_in_apk(work)
    sys.exit(result)


if __name__ == "__main__":
    main()
