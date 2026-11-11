# Changelog

## [0.4.0](https://github.com/AbdurahmanAlmehdi/jidr/compare/v0.3.1...v0.4.0) (2026-11-05)


### Features

* **backend:** -writable-system launch option ([f4fc850](https://github.com/AbdurahmanAlmehdi/jidr/commit/f4fc85005b3fb2452a15cc460c1c516aea38c837))
* **backend:** parse WAV/PCM clips and stream them to injectAudio ([#70](https://github.com/AbdurahmanAlmehdi/jidr/issues/70)) ([822cbcd](https://github.com/AbdurahmanAlmehdi/jidr/commit/822cbcd180a2d0144fa3335e560e29ef243d4c8c))
* **cli:** add `jidr root auto-grant <device> on|off` ([d0115e1](https://github.com/AbdurahmanAlmehdi/jidr/commit/d0115e1c7640197418fdab5ef03a42c8208074ac))
* **cli:** add `jidr root auto-grant <device> true|false` ([63d85b8](https://github.com/AbdurahmanAlmehdi/jidr/commit/63d85b805e45a816992926be13b3e8523f2552a4))
* **cli:** add jidr shell, an interactive shell and script runner ([56695a6](https://github.com/AbdurahmanAlmehdi/jidr/commit/56695a69ce78b9e81f83340d7a10e9e06d96b943))
* **cli:** add jidr shell, an interactive shell and script runner ([8fb44f5](https://github.com/AbdurahmanAlmehdi/jidr/commit/8fb44f5cbbb39ef4e30d40d4328bd1d25380b097)), closes [#73](https://github.com/AbdurahmanAlmehdi/jidr/issues/73)
* **cli:** jidr audio inject and jidr audio stop ([#70](https://github.com/AbdurahmanAlmehdi/jidr/issues/70)) ([108fcbe](https://github.com/AbdurahmanAlmehdi/jidr/commit/108fcbe07f25ee3d07eb90d2d92b13a795c30083))
* **cli:** jidr root writable-system &lt;device&gt; on|off ([1c650a2](https://github.com/AbdurahmanAlmehdi/jidr/commit/1c650a278c0fa8b41d4be62109a26837ca16cac3))
* **gui:** fullscreen player ([c61cf4a](https://github.com/AbdurahmanAlmehdi/jidr/commit/c61cf4aecb8d2bb4056670bdedb9ac3a2309cb11))
* **gui:** fullscreen player ([4e5ae6b](https://github.com/AbdurahmanAlmehdi/jidr/commit/4e5ae6b39dd740e8c96659e8924e2f359f307e74)), closes [#71](https://github.com/AbdurahmanAlmehdi/jidr/issues/71)
* inject an audio file as the device's microphone ([49cb114](https://github.com/AbdurahmanAlmehdi/jidr/commit/49cb114454f9d70d14acd27578db63282ac26d3e))
* **root:** writable /system for adb-root devices ([50609f6](https://github.com/AbdurahmanAlmehdi/jidr/commit/50609f6355f4f4763a68d6c3521a4d515ae62c4e))
* **root:** writable /system for adb-root devices ([d5fada0](https://github.com/AbdurahmanAlmehdi/jidr/commit/d5fada0d518618cc672d1cdd817f2ec63955cd41)), closes [#25](https://github.com/AbdurahmanAlmehdi/jidr/issues/25)
* **root:** writable /system needs API 31 or later ([5ae8181](https://github.com/AbdurahmanAlmehdi/jidr/commit/5ae81819207d4e1c08bab987349dacc18dacfad4)), closes [#25](https://github.com/AbdurahmanAlmehdi/jidr/issues/25)
* **skills:** add the jidr-root skill ([25490fa](https://github.com/AbdurahmanAlmehdi/jidr/commit/25490fa1ad9cd15b467e17a2e570163ab1a5258b))
* **skills:** add the jidr-root skill ([c1c1781](https://github.com/AbdurahmanAlmehdi/jidr/commit/c1c17819c29b24a6a1710f1c75b7646c7dbef299)), closes [#69](https://github.com/AbdurahmanAlmehdi/jidr/issues/69)
* **widgets:** audio.inject and audio.stop ([#70](https://github.com/AbdurahmanAlmehdi/jidr/issues/70)) ([08f5ab0](https://github.com/AbdurahmanAlmehdi/jidr/commit/08f5ab0d4a6b473b943f6f5f35f35cb0ff50071e))


### Bug Fixes

* **backend:** retry an audio injection the emulator refuses as busy ([6b66c84](https://github.com/AbdurahmanAlmehdi/jidr/commit/6b66c847ce23d1f7b775d3bc93a9e64e22cef27b)), closes [#70](https://github.com/AbdurahmanAlmehdi/jidr/issues/70)
* **backend:** use as_chunks for fixed-size WAV samples ([#70](https://github.com/AbdurahmanAlmehdi/jidr/issues/70)) ([8239f51](https://github.com/AbdurahmanAlmehdi/jidr/commit/8239f51013ec951f6a2985bde7cc80a97b2e3c90))
* **daemon:** flush guest storage before a graceful stop ([b9f4904](https://github.com/AbdurahmanAlmehdi/jidr/commit/b9f4904d893f6c5db49f27847f5d5fcdb6baec27))
* **daemon:** flush guest storage before a graceful stop ([2efe537](https://github.com/AbdurahmanAlmehdi/jidr/commit/2efe5372371b4a81fc4e29e9fbc86f4451f4bc63)), closes [#79](https://github.com/AbdurahmanAlmehdi/jidr/issues/79)
* **root:** reboot once when adb remount leaves /system read-only ([ef870ac](https://github.com/AbdurahmanAlmehdi/jidr/commit/ef870ac1fde4ebecbf9623475c94a17359f650a8)), closes [#25](https://github.com/AbdurahmanAlmehdi/jidr/issues/25)
* **ui:** fire ⌘ shortcuts on macOS ([728475e](https://github.com/AbdurahmanAlmehdi/jidr/commit/728475ece48a40e120806ca03546e4fa3a8f2876))
* **ui:** fire ⌘ shortcuts on macOS ([b2c4a88](https://github.com/AbdurahmanAlmehdi/jidr/commit/b2c4a88d08d041c2dd36ae08101709be33dc693f))

## [0.3.1](https://github.com/AbdurahmanAlmehdi/jidr/compare/v0.3.0...v0.3.1) (2026-11-03)


### Bug Fixes

* **release:** retry hdiutil create when the image is busy ([f6f0be4](https://github.com/AbdurahmanAlmehdi/jidr/commit/f6f0be4fb82e0f0f969fc320347a843c64bd7b38))
* **release:** retry hdiutil create when the image is busy ([3277063](https://github.com/AbdurahmanAlmehdi/jidr/commit/3277063a8c96b7a4ee392cae5ed8d657c65a086b))

## [0.3.0](https://github.com/AbdurahmanAlmehdi/jidr/compare/v0.2.0...v0.3.0) (2026-11-03)


### Features

* **daemon:** bases, pool leases and a reaper for agents (Phase 13d, D42) ([c780dc5](https://github.com/AbdurahmanAlmehdi/jidr/commit/c780dc5355797bbf8d2ee51913d307ed612a9dbb))
* **daemon:** hibernate, auto-hibernate and a working quick-boot save (Phase 13c, D42) ([a186692](https://github.com/AbdurahmanAlmehdi/jidr/commit/a18669279bf4cc6c3205e2d2eec28a548575cf03))
* **daemon:** host RAM budget, lean profile and host.budget (Phase 13a, D42) ([b06e0a6](https://github.com/AbdurahmanAlmehdi/jidr/commit/b06e0a6123da5317f248db6493f2d382580b45dc))
* many devices on one machine (D42, Phases 13a–13d) ([48a8289](https://github.com/AbdurahmanAlmehdi/jidr/commit/48a8289d64a28674fc1f34d4ae20082f32cba7ab))
* **root:** trust CA certificates as system certs on Full-root devices ([68c79b8](https://github.com/AbdurahmanAlmehdi/jidr/commit/68c79b8a01d1c3ece68e351508c606dd6e023427))
* **root:** trust CA certificates as system certs on Full-root devices ([0bf3c20](https://github.com/AbdurahmanAlmehdi/jidr/commit/0bf3c20b4887c1fe634f8937dbfdbd6e94b19450))
* **sdk:** snapshot-keeping copy-on-write clones (Phase 13b, D42) ([3fe675d](https://github.com/AbdurahmanAlmehdi/jidr/commit/3fe675da6ce7cf71da6fbd78700296f836721899))
* **site:** take the download's version, size and checksum from GitHub ([3f5f8d3](https://github.com/AbdurahmanAlmehdi/jidr/commit/3f5f8d3cb1356ca9455807447fbcc542e93f9862))
* **site:** take the download's version, size and checksum from GitHub ([e287b93](https://github.com/AbdurahmanAlmehdi/jidr/commit/e287b9304b828ec634625d16882b21b4b757ae70))
* **skills:** jidr and jidr-app-testing agent skills with xtask skills-check (D43) ([cec4fcb](https://github.com/AbdurahmanAlmehdi/jidr/commit/cec4fcb6b69f0e05744ff27c759d46552399b351))
* **skills:** jidr and jidr-app-testing agent skills with xtask skills-check (D43) ([db26b30](https://github.com/AbdurahmanAlmehdi/jidr/commit/db26b30a3baaa7fca215bdacac8c5134aa71188f))
* **widgets:** set and clear a device's HTTP proxy ([8b44aab](https://github.com/AbdurahmanAlmehdi/jidr/commit/8b44aab0e957d6629e4c06c7569ea3bbebf41d94))
* **widgets:** set and clear a device's HTTP proxy ([340e11d](https://github.com/AbdurahmanAlmehdi/jidr/commit/340e11d9778c5f572c61cc678201c3010a3d54de))


### Bug Fixes

* CI and test failures after the merge batch ([ebb571d](https://github.com/AbdurahmanAlmehdi/jidr/commit/ebb571df793c07fc2ae972cd3759236ebaa5d734))
* **daemon:** save quick-boot state when stopping a resumed run ([4566dde](https://github.com/AbdurahmanAlmehdi/jidr/commit/4566ddecbd33251a1b80a6cb166720696e3d8bdb))
* Full-root su hang recovery, Magisk version text, new-device auto start ([ba9cb20](https://github.com/AbdurahmanAlmehdi/jidr/commit/ba9cb20dfc764cb20b98ae7d475dac422d993daa))
* **packaging:** keep the template placeholders out of the cask's comment ([0a1a9f6](https://github.com/AbdurahmanAlmehdi/jidr/commit/0a1a9f66758b3f7d3a475369c67b5aa5b8c35330))
* **root:** recover a shell su that hangs right after the Full-root grant ([ff1beeb](https://github.com/AbdurahmanAlmehdi/jidr/commit/ff1beeb9160581424c2f71a58ca6e92bee6bcbf9))
* **root:** report the Magisk version without magisk -v's variant suffix ([a69eb44](https://github.com/AbdurahmanAlmehdi/jidr/commit/a69eb4474d929f30fca9b1a19843f1dd0b8de039))
* **root:** take one Zygote pid when pidof prints several ([a344019](https://github.com/AbdurahmanAlmehdi/jidr/commit/a3440198b1d10e5f8b551168bafc5283ca62b5ea))
* **site:** run the site tests on Node 22 ([e4b0892](https://github.com/AbdurahmanAlmehdi/jidr/commit/e4b08925970dbcfc4e066aa8b53570615e1d6171))
* **ui:** map player touches through the display rotation ([b29b53a](https://github.com/AbdurahmanAlmehdi/jidr/commit/b29b53a3dd69565258e629a4b5d23f178d4bd823))
* **ui:** map player touches through the display rotation ([2b01155](https://github.com/AbdurahmanAlmehdi/jidr/commit/2b01155dbabf3028864852729834b66d93628fc5)), closes [#51](https://github.com/AbdurahmanAlmehdi/jidr/issues/51)
* **ui:** start a new Full-root device's setup when the device list arrives ([edcde01](https://github.com/AbdurahmanAlmehdi/jidr/commit/edcde01fae1fb77dc168b6af8d51b9d1aa107e2a))

## [0.2.0](https://github.com/AbdurahmanAlmehdi/jidr/compare/v0.1.0...v0.2.0) (2026-10-18)


### Features

* **site:** blog with two technical guides ([7264e14](https://github.com/AbdurahmanAlmehdi/jidr/commit/7264e1464fcb3f4b632f0c11bb9f5f8397426600))
* **site:** show the brew install and upgrade lines now the cask is in the tap ([3a30c65](https://github.com/AbdurahmanAlmehdi/jidr/commit/3a30c652052c8a192ba313898d445a87f7a62a14))
* **site:** switch jidr.ly to the launch page for v0.1.0 ([094c642](https://github.com/AbdurahmanAlmehdi/jidr/commit/094c642b5c61b338b2a98d10b1b073b8a42ed6ed))
* **site:** switch jidr.ly to the launch page for v0.1.0 ([3fcc5ad](https://github.com/AbdurahmanAlmehdi/jidr/commit/3fcc5ad4ba4ff58779ebb8adf8e07965cdf57a11))
* **ui:** Check for updates compares with the latest GitHub release ([45ee1d6](https://github.com/AbdurahmanAlmehdi/jidr/commit/45ee1d6d63b20673d1c4e83b0695b536e57c5311))
* **ui:** Check for updates compares with the latest GitHub release ([f907b68](https://github.com/AbdurahmanAlmehdi/jidr/commit/f907b685204da59e3ca8c1ebd13480a32d3aa646)), closes [#44](https://github.com/AbdurahmanAlmehdi/jidr/issues/44)

## 0.1.0 (2026-10-18)


### Features

* **backend:** AOSP emulator launch, boot wait and graceful stop (Phase 3b) ([80e15e7](https://github.com/AbdurahmanAlmehdi/jidr/commit/80e15e722962da2149ae69a676d314af2758a27c))
* **backend:** camera options in LaunchOptions and Device.launch (R7, D36) ([732662f](https://github.com/AbdurahmanAlmehdi/jidr/commit/732662f8c40c8de688f64ce6e54b416ad7c84020))
* **backend:** crash detection, re-adopt, double-launch refusal, console client (Phase 3c) ([9c1dfaa](https://github.com/AbdurahmanAlmehdi/jidr/commit/9c1dfaa3203245edb8cce08f0e2de0996b0dccd3))
* build-prop spoofing for Full-root devices (Phase 9f) ([eef1509](https://github.com/AbdurahmanAlmehdi/jidr/commit/eef1509c28987e954f2b611140efb54f47925db7))
* **cli,sdk:** download cache, uninstall --purge and device log bundles (Phase 5b) ([683c2c5](https://github.com/AbdurahmanAlmehdi/jidr/commit/683c2c5ebd83461a94c150f9d6f9941b64e5d33b))
* **cli:** jidr command line over the daemon protocol (Phase 5a) ([b8abad3](https://github.com/AbdurahmanAlmehdi/jidr/commit/b8abad3a59730a41d953daeb664572471fe5c6b2))
* **cli:** register the root provider ([83e4ea3](https://github.com/AbdurahmanAlmehdi/jidr/commit/83e4ea355d579c13b9743bf2c8b2b24e3c761bda))
* **cpio:** ramdisk cpio reader/writer with gzip, lz4-legacy and xz (Phase 9b) ([452a1fa](https://github.com/AbdurahmanAlmehdi/jidr/commit/452a1faedae63fe88f216ac973019a846eb89561))
* daemon settings, byte progress, pristine ramdisk hash, full command reference (B4, B5, G14, G16) ([ca9df9e](https://github.com/AbdurahmanAlmehdi/jidr/commit/ca9df9e53d48bcc905567a83c74683cc49ca8d26))
* daemon settings, byte progress, pristine ramdisk hash, full command reference (B4, B5, G14, G16) ([9203b99](https://github.com/AbdurahmanAlmehdi/jidr/commit/9203b998997f568652f9eed9e6f4d3da1ed5424d))
* **daemon,client:** jobs, events, idle exit and the jidr-client library (Phase 4b) ([e4b1d08](https://github.com/AbdurahmanAlmehdi/jidr/commit/e4b1d0816d3ffbd2701f7fd7a7227970b3b431ab))
* **daemon:** freeze control protocol v0.1 with generated docs and fuzzing (Phase 4c) ([e7f591d](https://github.com/AbdurahmanAlmehdi/jidr/commit/e7f591de20bd4fcc50874ef22206f56ac7a464f5))
* **daemon:** JSON-RPC daemon core, provider contract and device commands (Phase 4a) ([d56990c](https://github.com/AbdurahmanAlmehdi/jidr/commit/d56990c6cbf345643a1e69917fc243180802f663))
* **jidr:** register the widgets provider in jidr daemon (G6) ([83cb597](https://github.com/AbdurahmanAlmehdi/jidr/commit/83cb597bf993c6be5a5c06b1e27da1f9be6f1c90))
* **jidr:** register the widgets provider in jidr daemon (G6) ([d21876f](https://github.com/AbdurahmanAlmehdi/jidr/commit/d21876f8062e5c9b4e087b73636d2a166aa05a31))
* **mcp:** jidr mcp server with generic and typed device tools (Phase 6) ([1a1e20f](https://github.com/AbdurahmanAlmehdi/jidr/commit/1a1e20f627d72201ac1ce2c6855a2a1dcc51124c))
* **proto,adb:** emulator gRPC bindings with vendored fallback and adb wrapper (Phase 3a) ([485ed78](https://github.com/AbdurahmanAlmehdi/jidr/commit/485ed780c6834184102dcbdcf2774a7c7c58663e))
* **release:** Homebrew cask and MCP bundle with every release ([dc6af6d](https://github.com/AbdurahmanAlmehdi/jidr/commit/dc6af6d4c2e275f050d112899874f54fbb494ecf))
* **release:** publish the Homebrew cask and an MCP bundle with every release ([521e05d](https://github.com/AbdurahmanAlmehdi/jidr/commit/521e05debca849667a77a9dfd3f97737057750f9))
* **release:** unsigned macOS app bundle, DMG, checksum and forbidden-content scan (Phase 12a) ([e13a935](https://github.com/AbdurahmanAlmehdi/jidr/commit/e13a935cde2bd8cef0024173409286abc8556ced))
* **root:** adb root mode with live switching, boot hook and verification (Phase 9c) ([285543f](https://github.com/AbdurahmanAlmehdi/jidr/commit/285543fe7093f39e4b4b481076687afe1ed1b34a))
* **root:** device.props.get/set — build-prop spoofing through the Magisk ramdisk (9f) ([c7d3da6](https://github.com/AbdurahmanAlmehdi/jidr/commit/c7d3da673319977a00cdc660c65dc8327f6ab9a5))
* **root:** Magisk mode (Phase 9d) ([cc9d1e9](https://github.com/AbdurahmanAlmehdi/jidr/commit/cc9d1e904324a1794db039fcaea152e83d665457))
* **root:** root.auto_grant for D22 auto-grant all (G15) ([418b583](https://github.com/AbdurahmanAlmehdi/jidr/commit/418b583a95582268c9a96432d94f9fae11ce6878))
* **root:** root.auto_grant for D22 auto-grant all (G15) ([3834178](https://github.com/AbdurahmanAlmehdi/jidr/commit/3834178c4c93034be9f96f8e9ff2fea7dd0700c6))
* **sdk:** core model, error codes, native SDK installer and AVD lifecycle (Phase 2) ([93d494c](https://github.com/AbdurahmanAlmehdi/jidr/commit/93d494c67f5743ac81f2d6fb885be505712a1af2))
* **site:** coming-soon landing page for jidr.ly (Phase 11a) ([92f1185](https://github.com/AbdurahmanAlmehdi/jidr/commit/92f11850fc6f5dc00e9c883665c0454442c21b94))
* **site:** go live as the coming-soon page with links that work ([991f78e](https://github.com/AbdurahmanAlmehdi/jidr/commit/991f78e41a71f1f140409269ecb2aada03637cf1))
* **site:** launch page with download, agents, CI and docs, plus Arabic RTL (Phase 11b) ([f0858e9](https://github.com/AbdurahmanAlmehdi/jidr/commit/f0858e91c5f9a32c78f826d86c013a9cc6e32324))
* **site:** rebuild the coming-soon page to the Workbench design (Phase 11a, D41) ([8a318ea](https://github.com/AbdurahmanAlmehdi/jidr/commit/8a318ea0c609421dcfe495a94e7f3430b4e73e62))
* **site:** use the brand kit for favicon, header symbol and social card (Phase 11a, D39) ([816e53f](https://github.com/AbdurahmanAlmehdi/jidr/commit/816e53f4b0a2a5b7e1fcf10c77f46116f828a804))
* **spike:** display + input spike passes, ADR-0001 frame transport (Phase 1) ([a09c8a2](https://github.com/AbdurahmanAlmehdi/jidr/commit/a09c8a20dd46482c18dc74dfd1a99fe55e939af9))
* **ui,widgets:** player window with direct gRPC frames, input, drag-drop and logcat (Phase 7c) ([463bda3](https://github.com/AbdurahmanAlmehdi/jidr/commit/463bda3d4577703215666bdb441e7687cef71130))
* **ui:** Build identity form for Full-root devices; docs for 9f ([4ca2a9b](https://github.com/AbdurahmanAlmehdi/jidr/commit/4ca2a9b7c0ea01ec1da5029af314f7fe9752f34a))
* **ui:** first-run setup, create/edit sheet, settings and command palette (Phase 7b) ([c877981](https://github.com/AbdurahmanAlmehdi/jidr/commit/c877981cbaa49dc9eaa7b80892bcaca90ff27e18))
* **ui:** root UX for Phase 9e (src/root) ([e1f8597](https://github.com/AbdurahmanAlmehdi/jidr/commit/e1f8597bd9bbd0e40f3c419a75691c3c69503d00))
* **ui:** root UX slots for Phase 9e (gap B8) ([a70ee36](https://github.com/AbdurahmanAlmehdi/jidr/commit/a70ee36a981e0618db7ee3e21f97b21da264a7db))
* **ui:** Workbench shell, generated theme, daemon bridge and snapshot harness (Phase 7a) ([a9de977](https://github.com/AbdurahmanAlmehdi/jidr/commit/a9de9773104292d4182a419e0ff7c78702c5c78f))
* **widgets,ui:** battery, sensors, network, phone, controls, dev tools, clipboard, fingerprint and camera widgets (Phase 8a) ([d9daeb4](https://github.com/AbdurahmanAlmehdi/jidr/commit/d9daeb47ce1f511f79e422af2e9e78ea83abdf32))
* **widgets,ui:** GPS fix and GPX/KML route replay with an OSM map panel (Phase 8b) ([05fabf5](https://github.com/AbdurahmanAlmehdi/jidr/commit/05fabf5d38e46835b24662a275610d3f37d4d917))
* **widgets,ui:** PNG screenshots and MP4/H.264 recording via VideoToolbox (Phase 10a) ([3fd4314](https://github.com/AbdurahmanAlmehdi/jidr/commit/3fd43140ccb7fcc5ef47e37b50c2a72bc80d37ec))
* **widgets,ui:** snapshot manager with root-mode tags and cross-mode refusal (Phase 10b) ([d702a26](https://github.com/AbdurahmanAlmehdi/jidr/commit/d702a26ca9c835a23ccd10f100e386f159d70812))


### Bug Fixes

* **backend:** launch the emulator with -sysdir for the device's image ([2c5e54b](https://github.com/AbdurahmanAlmehdi/jidr/commit/2c5e54b4ed2522d312699effc3df7793e09a5335))
* **backend:** launch the emulator with -sysdir for the device's image ([f70a090](https://github.com/AbdurahmanAlmehdi/jidr/commit/f70a0905fd6e36580a95e4cc280a0cd1af853cb2))
* **client:** never spawn the GUI as the daemon from Jidr.app ([7c5c8d3](https://github.com/AbdurahmanAlmehdi/jidr/commit/7c5c8d3fffbeacc493a84019c86669751c629cbb))
* **client:** never spawn the GUI as the daemon from Jidr.app ([fb93687](https://github.com/AbdurahmanAlmehdi/jidr/commit/fb93687e5f71a1506b77b70945432e25ee362949))
* **design:** Arabic landing type sizes in tokens.json (G13) ([f6d1462](https://github.com/AbdurahmanAlmehdi/jidr/commit/f6d14622c72c3bb1e04151341cd79086dbc2a2f1))
* **root:** back up the stock init so Magisk devices boot ([820df53](https://github.com/AbdurahmanAlmehdi/jidr/commit/820df53cf25692296665115e88e3d92e26f087fe))
* **root:** never leak the e2e emulator on failure, panic or a killed test run ([757f087](https://github.com/AbdurahmanAlmehdi/jidr/commit/757f087933489c9eae1e201744cd9d627bcb6c73))
* **root:** one guarded adbd switch for adb and Magisk modes; kick hung transports ([e610d7b](https://github.com/AbdurahmanAlmehdi/jidr/commit/e610d7b3b100cea1e10c31f36d1dba62ff25a5f3))
* **root:** wait and retry once when adb root hits a closed transport ([4fdea93](https://github.com/AbdurahmanAlmehdi/jidr/commit/4fdea937a4a130e3127de3aeeb04b5e34ed2067b))
* **root:** wait for adb after a cold boot or an adbd restart ([9a25719](https://github.com/AbdurahmanAlmehdi/jidr/commit/9a2571962231b4dc4043b902d44a6fa0baf2bf88))
* **root:** wait for adb after a cold boot or an adbd restart ([143c510](https://github.com/AbdurahmanAlmehdi/jidr/commit/143c5101284f2c5029d97be1be0923af4a4c4798))
* **scripts:** accept an absolute CARGO_TARGET_DIR in the e2e scripts ([8fc9680](https://github.com/AbdurahmanAlmehdi/jidr/commit/8fc96802c8acf1a16a9b58adcab544e51ce24a79))
* **site:** take the Arabic landing type sizes from jidr.css until tokens.json has them (G13) ([4d284af](https://github.com/AbdurahmanAlmehdi/jidr/commit/4d284af36b224f4254c00180257ca9994003873a))
* **ui:** room for the Play Store card's three-line note ([5395e6a](https://github.com/AbdurahmanAlmehdi/jidr/commit/5395e6a6c5b5686f2b7894ce5ee31820582afca2))
* **ui:** show the auto-grant toggle as disabled without root.auto_grant ([7dff8ae](https://github.com/AbdurahmanAlmehdi/jidr/commit/7dff8aec4fd1885fb1041faa6424139f37dd5b2f))
* **widgets,ui:** snapshot path traversal, AppleScript injection, stuck recordings, per-device record state ([64e78ab](https://github.com/AbdurahmanAlmehdi/jidr/commit/64e78abbb985f2c36a9915dcdf9e62399750b4aa))
* **widgets:** app.launch starts the launcher activity with am start -W and confirms it ([3564e70](https://github.com/AbdurahmanAlmehdi/jidr/commit/3564e70481d55ac9d85512c70fb4ba3a425199c7))
* **widgets:** build on Linux and keep snapshot tags when the emulator renames folders ([c0c1832](https://github.com/AbdurahmanAlmehdi/jidr/commit/c0c1832b38824014700a17001aa7159f4db57e71))
* **widgets:** network.set checks the radio took gsm data; retry mobile waits ([b3d14ab](https://github.com/AbdurahmanAlmehdi/jidr/commit/b3d14abd5ff3cb9b784bbb4436dafc3c1b90a66f))
* **widgets:** network.set checks the radio took gsm data; retry mobile waits ([c993da8](https://github.com/AbdurahmanAlmehdi/jidr/commit/c993da871fb767d3f2eeee2408a998c6c713f43c))
* **widgets:** snapshot.load waits until the device answers adb again ([1074699](https://github.com/AbdurahmanAlmehdi/jidr/commit/10746993cba4235867fedbe8aad8b555ace0b1a3))
* **widgets:** switch mobile data at the emulated radio; wait for voice service before calling ([a5c7dc7](https://github.com/AbdurahmanAlmehdi/jidr/commit/a5c7dc702eecef1276738c363bd79e7bb771213b))
