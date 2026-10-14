#!/usr/bin/env bash
# Builds a tiny test app, ly.jidr.hello ("Hello from Jidr"), with the SDK's
# build tools (used read-only). Usage: build-hello-apk.sh <out.apk>
# Env: JIDR_TEST_SDK (default ~/Library/Android/sdk). Never shipped.
set -euo pipefail
out="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
sdk="${JIDR_TEST_SDK:-$HOME/Library/Android/sdk}"
bt="$(ls -d "$sdk"/build-tools/* | sort -V | tail -1)"
jar="$(ls -d "$sdk"/platforms/android-* | sort -V | tail -1)/android.jar"
javac="${JAVAC:-$(command -v javac || echo "/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/javac")}"
keytool="$(dirname "$javac")/keytool"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/src/ly/jidr/hello" "$work/classes" "$work/dex"
cat > "$work/AndroidManifest.xml" <<'XML'
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="ly.jidr.hello">
  <application android:label="Jidr Hello">
    <activity android:name=".MainActivity" android:exported="true">
      <intent-filter>
        <action android:name="android.intent.action.MAIN"/>
        <category android:name="android.intent.category.LAUNCHER"/>
      </intent-filter>
    </activity>
  </application>
</manifest>
XML
cat > "$work/src/ly/jidr/hello/MainActivity.java" <<'JAVA'
package ly.jidr.hello;
public class MainActivity extends android.app.Activity {
    @Override protected void onCreate(android.os.Bundle b) {
        super.onCreate(b);
        android.widget.TextView t = new android.widget.TextView(this);
        t.setText("Hello from Jidr");
        t.setTextSize(28);
        setContentView(t);
    }
}
JAVA
"$javac" --release 11 -nowarn -classpath "$jar" -d "$work/classes" "$work/src/ly/jidr/hello/MainActivity.java"
"$bt/d8" --lib "$jar" --min-api 29 --output "$work/dex" $(find "$work/classes" -name '*.class')
"$bt/aapt2" link -o "$work/unsigned.apk" -I "$jar" --manifest "$work/AndroidManifest.xml" \
  --min-sdk-version 29 --target-sdk-version 31
(cd "$work/dex" && zip -q -u "$work/unsigned.apk" classes.dex)
"$bt/zipalign" -f 4 "$work/unsigned.apk" "$work/aligned.apk"
"$keytool" -genkeypair -keystore "$work/ks" -storepass jidrtest -keypass jidrtest -alias t \
  -keyalg RSA -keysize 2048 -validity 1 -dname "CN=Jidr test" >/dev/null 2>&1
"$bt/apksigner" sign --ks "$work/ks" --ks-pass pass:jidrtest --key-pass pass:jidrtest --out "$out" "$work/aligned.apk"
echo "$out"
