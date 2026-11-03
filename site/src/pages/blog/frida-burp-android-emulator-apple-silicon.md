---
layout: ../../layouts/Post.astro
title: "Frida and Burp Suite on an Android emulator on Apple Silicon (M1–M4), rooted with Magisk"
description: "A rooted Android 15 emulator on an M-series Mac with frida-server, a Burp proxy, Burp's CA as a system certificate and SSL pinning turned off, saved as a snapshot."
date: 2026-11-03
---

Testing an Android app usually means three things: run Frida on the device, send its traffic through Burp, and get past certificate pinning. On an M-series Mac, the arm64 system images from Google run natively and fast. What they lack is root, a way to trust Burp's CA as a system certificate, and a quick way back to a working setup.

This guide builds that lab with [Jidr](https://github.com/AbdurahmanAlmehdi/jidr), a free, open-source emulator manager for Apple Silicon Macs. Every command below was run on an M1 Pro with Android 15 (API 35, Google APIs, arm64), Magisk 30.7 and Frida 17.25.2. The proxy output is from mitmproxy 11 standing in for Burp; the Jidr side doesn't change. Test only apps you are allowed to test.

**You need Jidr 0.3.0 or later** for `jidr proxy` and `jidr cert` (check with `jidr --version`).

## 1. A rooted device

Install Jidr and an image, then create a device and give it Full root:

```bash
brew install --cask abdurahmanalmehdi/tap/jidr
jidr image install 35 --accept-licenses   # you accept Google's SDK licence
jidr device create Lab --image 35
jidr root set Lab magisk
```

```text
Created Lab  stopped  35:google_apis  root adb
[  …] root.set Cold booting
[  …] root.set Installing the Magisk app
[  …] root.set Setting up Magisk's environment
[  …] root.set Stopping before the cold boot
[  …] root.set Cold booting
[  …] root.set Granting the adb shell su access
[100%] root.set Verifying root
```

That took two minutes. Jidr patches Magisk into a per-device copy of the ramdisk, boots it, grants `su` to the shell and checks that `su -c id` returns uid 0. The [rooting post](/blog/root-android-emulator-apple-silicon/) explains each step. Use a Google APIs image (`35` means `35:google_apis`): Google Play images are production builds that need a tap on the device to grant `su`.

The commands below also need `adb` on your PATH. `brew install --cask android-platform-tools` works, or use the `platform-tools` folder of the SDK that `jidr sdk status` lists.

On a new device, the root setup can leave Wi-Fi with no saved network: no Wi-Fi icon, and `adb shell ip route` prints nothing. If so, connect it to the emulator's virtual access point once. The network is saved from then on:

```bash
adb shell cmd wifi connect-network AndroidWifi open
```

## 2. frida-server

`frida-server` must have the same version as the Frida on your Mac, and the device needs the `android-arm64` build:

```bash
frida --version
```

```text
17.25.2
```

Download that version from [Frida's releases](https://github.com/frida/frida/releases), then push it and start it as root:

```bash
curl -LO https://github.com/frida/frida/releases/download/17.25.2/frida-server-17.25.2-android-arm64.xz
xz -d frida-server-17.25.2-android-arm64.xz
adb push frida-server-17.25.2-android-arm64 /data/local/tmp/frida-server
adb shell chmod 755 /data/local/tmp/frida-server
adb shell "su -c 'nohup /data/local/tmp/frida-server >/dev/null 2>&1 &'"
```

Start it this way rather than with `frida-server -D`: the daemon keeps `adb shell` attached, so the command never returns. Then check from the Mac:

```bash
frida-ps -Ua
```

```text
 PID  Name             Identifier
----  ---------------  ---------------------------------------
2070  Camera           com.android.camera2
2912  Chrome           com.android.chrome
1339  Google           com.google.android.googlequicksearchbox
1339  Google           com.google.android.googlequicksearchbox
2076  Magisk           com.topjohnwu.magisk
3181  Messages         com.google.android.apps.messaging
3015  Personal Safety  com.google.android.apps.safetyhub
 995  Settings         com.android.settings
```

## 3. The proxy

Burp listens on `127.0.0.1:8080` by default. Inside the emulator, `10.0.2.2` is the Mac's `127.0.0.1`, so the default listener works as it is:

```bash
jidr proxy set Lab 8080
jidr proxy show Lab
```

```text
HTTP proxy: 10.0.2.2:8080
HTTP proxy: 10.0.2.2:8080
```

The port alone means `10.0.2.2`; give `host:port` for a proxy elsewhere. This sets Android's global `http_proxy`, so it takes effect at once and covers every app that uses the system proxy, on Wi-Fi and mobile data. It stays set across restarts and snapshots. To stop proxying:

```bash
jidr proxy clear Lab
```

```text
No HTTP proxy.
```

At this point plain HTTP shows up in Burp, but HTTPS fails: apps don't trust Burp's CA yet.

## 4. Burp's CA as a system certificate

Apps that target Android 7 or later ignore CAs that the user installs. They trust only the system store, unless the app's network security config opts in to user CAs. So the CA has to go in the system store, which needs root.

On Android 14 and later, that store moved. The CAs now come from the Conscrypt APEX, `/apex/com.android.conscrypt/cacerts`, which is read-only and is mounted into each app's namespace from Zygote. Copying a file into `/system/etc/security/cacerts`, the old trick, no longer reaches apps.

Export Burp's CA in DER format (Proxy › Proxy settings › Import / export CA certificate; mitmproxy keeps its CA in `~/.mitmproxy/`), then:

```bash
jidr cert install Lab cacert.der
```

```text
[  …] root.cacert.install Stopping before the cold boot
[  …] root.cacert.install Cold booting
[100%] root.cacert.install Verifying the system CA store
mitmproxy (c8750f0d.0) is now a trusted system certificate.
```

Jidr writes its own small Magisk module. On API 34 and later, its `post-fs-data.sh`, which Magisk runs before Zygote starts, copies the APEX store and your CA into a tmpfs and bind-mounts it over `/apex/com.android.conscrypt/cacerts`. Zygote and every app it forks inherit the mount. On API 29–33 the module puts the CA in `/system/etc/security/cacerts` through Magisk's overlay. Either way the change applies at boot, so `install` cold boots the device, which took about 80 seconds here.

It then reads the store as Zygote sees it and compares the certificate byte for byte. You can run that check yourself at any time:

```bash
jidr cert verify Lab
jidr cert list Lab
```

```text
FILE        SUBJECT    TRUSTED
c8750f0d.0  mitmproxy  yes
System store: /apex/com.android.conscrypt/cacerts
```

`verify` exits with code 7 if a CA is missing from the store, so it works in scripts. To take the CA out again (this cold boots too):

```bash
jidr cert remove Lab c8750f0d.0   # or --all
```

```text
Removed c8750f0d.0.
```

## 5. SSL pinning

With the CA trusted, apps that don't pin show their HTTPS in Burp. To see what happens with pinning, try HTTP Toolkit's [SSL pinning demo](https://github.com/httptoolkit/android-ssl-pinning-demo), an Apache-2.0 test app with one button per pinning method:

```bash
curl -LO https://github.com/httptoolkit/android-ssl-pinning-demo/releases/download/v1.7.1/pinning-demo.apk
jidr install Lab pinning-demo.apk
```

"Unpinned request" turns green, and the proxy shows it:

```text
127.0.0.1:64500: GET https://testserver.host/
              << 200 OK 48.2k
```

"Config-pinned request" (pins in `network_security_config.xml`) and "OkHttp pinned request" (OkHttp's `CertificatePinner`) stay red. The pins are for the real server's keys, and the CA that signs what the app sees is Burp's.

This Frida script switches both checks off. The chain is already trusted, so it only has to skip the pin comparisons:

```js
// unpin.js: turn off certificate pinning in apps you are allowed to test.
// The CA is already a trusted system cert, so only the pin checks need to go.
Java.perform(() => {
  // Pins from res/xml/network_security_config.xml
  const NstM = Java.use("android.security.net.config.NetworkSecurityTrustManager");
  NstM.checkPins.implementation = function (chain) {
    console.log("[unpin] network security config pins skipped");
  };

  // OkHttp 3 and 4: CertificatePinner
  try {
    const Pinner = Java.use("okhttp3.CertificatePinner");
    Pinner.check.overloads.forEach((o) => {
      o.implementation = function (host) {
        console.log(`[unpin] OkHttp pins skipped for ${host}`);
      };
    });
    if (Pinner["check$okhttp"]) {
      Pinner["check$okhttp"].implementation = function (host) {
        console.log(`[unpin] OkHttp pins skipped for ${host}`);
      };
    }
  } catch (e) {
    console.log("[unpin] no OkHttp in this app");
  }
});
```

Spawn the app with it:

```bash
frida -U -f tech.httptoolkit.pinning_demo -l unpin.js
```

```text
Spawned `tech.httptoolkit.pinning_demo`. Resuming main thread!
[unpin] network security config pins skipped
[unpin] network security config pins skipped
[unpin] OkHttp pins skipped for rsa8192--untrusted-root.testserver.host
```

Both buttons turn green, and the requests are in the proxy:

```text
127.0.0.1:64532: GET https://rsa2048--untrusted-root.testserver.host/
              << 200 OK 48.2k
127.0.0.1:64565: GET https://rsa8192--untrusted-root.testserver.host/
              << 200 OK 48.2k
```

The demo's pinned endpoints are served from a deliberately untrusted root, so the proxy has to accept that upstream certificate too (`mitmdump --ssl-insecure` here). A real app's servers don't need that.

Real apps pin in other places too: TrustKit, Volley, a custom `TrustManager`, native code. Each needs its own hook. objection's `android sslpinning disable` covers many of them, but objection 1.12.5 killed the app on spawn with Frida 17.25.2 in this test, so check that your versions work together.

## 6. Save the lab

Once the device is rooted, trusts the CA, has the proxy set and runs `frida-server`, save it:

```bash
jidr snapshot save Lab lab-ready
```

```text
[100%] snapshot.save Saving memory and disk
```

That took 15 seconds. A snapshot holds memory as well as disk, so loading it brings back a running `frida-server` along with everything else:

```bash
jidr snapshot load Lab lab-ready   # 4 s here
jidr proxy show Lab
jidr cert verify Lab
frida-ps -Ua
```

```text
[100%] snapshot.load Restoring
HTTP proxy: 10.0.2.2:8080
FILE        SUBJECT    TRUSTED
c8750f0d.0  mitmproxy  yes
System store: /apex/com.android.conscrypt/cacerts
 PID  Name             Identifier
----  ---------------  ---------------------------------------
1742  Camera           com.android.camera2
…
```

Jidr tags each snapshot with the device's root mode and refuses to load it under another one, so you can't restore a Magisk snapshot onto a stock ramdisk by accident.

## Limits

- **Play Integrity fails.** No emulator passes it, rooted or not, because it attests hardware and boot state that an emulator doesn't have. Magisk's DenyList hides root only from local checks. Test flows that depend on Play Integrity on a real device.
- **arm64 images only.** Jidr runs arm64-v8a images, natively. An app that ships native code only for x86 won't run.
- **Mac only.** Jidr needs an Apple Silicon Mac (M1 or later).
- **Only proxy-aware traffic.** The proxy catches apps that use Android's system proxy. HTTP/3 (QUIC) runs over UDP and doesn't go through an HTTP proxy.

## Further reading

- [CA certificates in the Jidr docs](https://github.com/AbdurahmanAlmehdi/jidr/blob/main/docs/root.md#ca-certificates-system-trust)
- [The CLI's HTTP proxy section](https://github.com/AbdurahmanAlmehdi/jidr/blob/main/docs/cli.md#http-proxy)
- [Frida's Android docs](https://frida.re/docs/android/)
- [How to root the Android emulator on an Apple Silicon Mac](/blog/root-android-emulator-apple-silicon/)
