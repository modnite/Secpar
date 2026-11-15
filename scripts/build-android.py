#!/usr/bin/env python3
"""Build the Secpar Android app.

The app is the pinned Moonlight Android source (upstream/pins.json) with Secpar's
screen in front of it: android-secpar/ holds the added files, and the changes to
Moonlight's own build file and manifest are made here, each checked so a new
upstream version cannot be patched wrongly without notice.

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


def edit(path: Path, pairs):
    text = path.read_bytes().decode("utf-8")
    for old, new in pairs:
        if text.count(old) != 1:
            sys.exit(f"{path.name}: expected exactly one {old!r}")
        text = text.replace(old, new)
    path.write_bytes(text.encode("utf-8"))


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

    edit(work / "app" / "build.gradle", [
        ('ndkVersion "29.0.14206865"', f'ndkVersion "{NDK}"'),
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

    gradle = work / ("gradlew.bat" if os.name == "nt" else "gradlew")
    sys.exit(subprocess.call([str(gradle), task, "--console=plain"], cwd=work))


if __name__ == "__main__":
    main()
