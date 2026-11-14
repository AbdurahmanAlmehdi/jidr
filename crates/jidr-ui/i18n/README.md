# Translating the Jidr app

The desktop app ships in English, Arabic (`ar`), Spanish (`es`), Simplified Chinese (`zh-Hans`) and Brazilian Portuguese (`pt-BR`) (issue #29). The language follows the Mac's preferred languages, and Settings › General › Language can override it. A change applies without restarting.

| File | What it is |
|---|---|
| `jidr-ui.pot` | Every string in the app. Generated; don't edit it. |
| `<lang>/LC_MESSAGES/jidr-ui.po` | One language. Edit the `msgstr` lines only. |

```bash
cargo xtask i18n           # after changing strings in the code: refresh the template and every catalog
cargo xtask i18n --check   # what CI runs: stale catalogs, missing or fuzzy translations, placeholders, kept terms, RTL isolation
```

`.slint` strings use `@tr("…")` and Rust strings use `tr!("…")` / `trn!("one", "other", n)`. Both read these catalogs. `build.rs` compiles them into the app, so a catalog change needs a rebuild. Each msgid has one translation everywhere it appears (there is no per-component context).

## Rules

- **Placeholders:** keep every `{}`, `{0}`, `{1}` and `{n}`. Reorder them if the sentence needs it: `{0}` and `{1}` refer to arguments by position. `{n}` is the count in a plural, and a plural form may spell the number out instead (Arabic dual, for example).
- **Keep as is:** `jidr` commands, `adb`, `Magisk`, `su -c id`, `logcat`, `MCP`, `CLI`, `SDK`, `API`, `APK`, `GPS`, `GPX`, `KML`, `JSON`, `Play Integrity`, `Google APIs`, `OpenStreetMap`, `Homebrew` and `brew`, `Claude`, paths such as `/sdcard/Download`, file names, flags such as `--root`, and every code span. The check enforces most of them. "Android", "Pixel" and "Play Store" stay as Google names them.
- **Product name:** "Jidr" stays Latin in Spanish, Chinese and Portuguese. Arabic writes جذر, as the site and the wordmark do. Where that collides with الجذر (root) in one sentence, rephrase so it stays clear.
- **Numbers:** Western digits (0–9) in every language, Arabic included. Sizes keep `GB` and `MB`.
- **Arabic is right to left.** Wrap every placeholder in FIRST STRONG ISOLATE and POP DIRECTIONAL ISOLATE (U+2068 `{}` U+2069), so a Latin device name, serial, path or number keeps its own direction. This is what `<bdi>` does in HTML, and the check requires it. Do the same around a Latin run that starts or ends with punctuation, such as `macOS 14+`.
- **Arabic plurals** have six forms: 0, 1, 2, 3–10, 11–99, and 100+ (the `Plural-Forms` header). Write each one naturally: «لا أجهزة», «جهاز واحد», «جهازان», «{n} أجهزة», «{n} جهازًا», «{n} جهاز».
- **Chinese** has one plural form. Use full-width punctuation (，。：（）) in prose, and keep half-width `·` as the separator between fields.
- **Spanish and Portuguese** run about 30% longer than English. On buttons, tabs, badges and pills, prefer the short form.
- **Platform terms:** use the words macOS and Android use in each language, for example Finder (访达 in Chinese), Settings (Ajustes in Spanish and Portuguese, 设置 in Chinese, الإعدادات in Arabic), and the Download folder.
- If you aren't sure about a translation, add a translator comment above the entry (`# unsure: …`) so a native speaker can check it.

## Glossary

| English | العربية | Español | 简体中文 | Português (Brasil) |
|---|---|---|---|---|
| root (the privilege) | الجذر | root | Root 权限 | root |
| Full root | جذر كامل | Root completo | 完整 Root | Root completo |
| Shell root | جذر Shell | Root de shell | Shell Root | Root de shell |
| Not rooted / No root | بلا جذر | Sin root | 未 Root | Sem root |
| Rooted (verified badge) | مُجذَّر | Con root | 已 Root | Com root |
| Set up root | إعداد الجذر | Configurar root | 设置 Root | Configurar root |
| Revert to unrooted | إزالة الجذر | Quitar root | 移除 Root | Remover root |
| verify / verified | التحقق / مُتحقَّق منه | verificar / verificado | 验证 / 已验证 | verificar / verificado |
| device | جهاز | dispositivo | 设备 | dispositivo |
| system image | صورة النظام | imagen del sistema | 系统映像 | imagem do sistema |
| emulator | المحاكي | emulador | 模拟器 | emulador |
| snapshot | لقطة | instantánea | 快照 | snapshot |
| Quick boot | الإقلاع السريع | Arranque rápido | 快速启动 | Inicialização rápida |
| cold boot | إقلاع بارد | arranque en frío | 冷启动 | inicialização a frio |
| boot / booting | إقلاع / جارٍ الإقلاع | arranque / arrancando | 启动 / 正在启动 | inicialização / inicializando |
| hibernate / hibernated | إسبات / في إسبات | hibernar / hibernado | 休眠 / 已休眠 | hibernar / hibernado |
| Running | قيد التشغيل | En ejecución | 运行中 | Em execução |
| Stopped | متوقف | Detenido | 已停止 | Parado |
| base (a device agents clone) | قاعدة | base | 基础设备 | base |
| clone | نسخة | clon | 克隆 | clone |
| lease / leased (an agent holds a clone) | حجز / محجوز | préstamo / prestado | 租用 / 已租用 | empréstimo / emprestado |
| pool | مجمّع | grupo | 池 | pool |
| agent (AI) | وكيل | agente | 智能体 | agente |
| RAM budget | ميزانية الذاكرة | presupuesto de RAM | 内存预算 | orçamento de RAM |
| widget panel | لوحة الأدوات | panel de herramientas | 工具面板 | painel de ferramentas |
| build identity (props) | هوية البناء | identidad de compilación | 构建标识 | identidade da build |
| CA certificate | شهادة CA | certificado de CA | CA 证书 | certificado de CA |
| Jobs (the background tasks list) | المهام | Tareas | 任务 | Tarefas |

When a term isn't here, use the one that Android and macOS use in that language.
