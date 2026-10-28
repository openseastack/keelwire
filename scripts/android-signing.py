#!/usr/bin/env python3
"""Add release signing to the generated Android project (CI only).

`tauri android init` regenerates gen/android, so the signing config is
patched in after every init. The keystore path and passwords are read from
environment variables at build time and never written to disk here:
ANDROID_KEYSTORE_PATH, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS,
ANDROID_KEY_PASSWORD.

Usage: android-signing.py path/to/app/build.gradle.kts
"""
import sys

path = sys.argv[1]
src = open(path, encoding="utf-8").read()

if 'signingConfigs' in src and 'ANDROID_KEYSTORE_PATH' in src:
    print("signing already configured")
    sys.exit(0)

block = '''    signingConfigs {
        create("release") {
            storeFile = file(System.getenv("ANDROID_KEYSTORE_PATH"))
            storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
            keyAlias = System.getenv("ANDROID_KEY_ALIAS")
            keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
        }
    }
'''

anchor = "    buildTypes {"
release = 'getByName("release") {'
if anchor not in src or release not in src:
    print(src)
    sys.exit(f"unexpected build.gradle.kts layout: '{anchor}' or '{release}' not found")

src = src.replace(anchor, block + anchor, 1)
i = src.index(release) + len(release)
src = src[:i] + '\n            signingConfig = signingConfigs.getByName("release")' + src[i:]
open(path, "w", encoding="utf-8").write(src)
print("release signing added to", path)
