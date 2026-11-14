# jidrtest_module

A tiny Magisk module (Apache-2.0) used by `crates/jidr-root/tests/module_emulator.rs`
and `flash_mode.rs`. It was written for Jidr from Magisk's public module guide
(module layout, `service.sh`, `system.prop`, `uninstall.sh`); no module code was
copied. The tests zip this folder's contents (not the folder) at run time.
