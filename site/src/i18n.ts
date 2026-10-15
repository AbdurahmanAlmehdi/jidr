// Page copy in English and Arabic (D33). The English is the design's copy
// (docs/design/screens/Landing.dc.html); the Arabic follows its RTL hero.
// Strings ending in `Html`, and feature bodies, carry inline markup and render
// with set:html.
//
// The app itself is English-only for the MVP (D33), so what depicts the app
// (the product shot, the Rooted badge, the agent transcript, commands) stays
// English on both pages.

export const locales = ["en", "ar"] as const;
export type Locale = (typeof locales)[number];

const en = {
  lang: "en",
  dir: "ltr",
  home: "/",
  ogLocale: "en_US",
  title: "Jidr: Android on your Mac, rooted in a minute",
  description: {
    soon: "Jidr (جذر) is a free, open-source Android emulator for Apple Silicon Macs with one-click rooted devices, a CLI and an MCP server for AI agents. Coming soon.",
    launch: "Jidr (جذر) is a free, open-source Android emulator for Apple Silicon Macs with one-click rooted devices, a CLI and an MCP server for AI agents.",
  },
  ogImageAlt: "Jidr جذر: the Jidr symbol, a root growing into a phone outline",
  skip: "Skip to content",
  nav: {
    label: "Site",
    features: "Features",
    root: "Root",
    agents: "Agents",
    docs: "Docs",
    star: "Star",
  },
  // The link to the other language, written in that language.
  other: { href: "/ar/", lang: "ar", label: "العربية" },
  hero: {
    pill: { soon: "Coming soon · free and open source", launch: "Free and open source" },
    titleHtml: "Android on your Mac.<br />Rooted in a minute.",
    lede: "Jidr creates and runs virtual Android phones and tablets on Apple Silicon. Pick Full root and Magisk is set up and verified for you. Desktop app, CLI and MCP server.",
    star: "Star on GitHub",
    sponsor: "Sponsor",
    download: "Download for Mac",
    github: "View on GitHub",
    meta: { soon: ["Free forever", "no account", "macOS 14+ on Apple Silicon"], launch: ["macOS 14+", "Apple Silicon"] },
    version: (v: string) => `v${v}`,
    brew: "or brew install --cask jidr",
  },
  shotLabel: "The Jidr app: device list, a phone running a delivery app, and the GPS panel",
  root: {
    eyebrow: "One-click root",
    title: "Pick Full root. Get a verified badge.",
    lead: "Jidr does the patching you would otherwise do by hand, then checks that su really works before it says Rooted.",
    stepsHtml: [
      "Download Magisk 30.7",
      "Patch the boot image, keep the original",
      "Reboot (about 40 s)",
      "Install the Magisk app",
      "Verify <code>su -c id</code>",
    ],
    step: (n: number) => `step ${n}`,
    failure: "If a step fails you see which one and why, with the log, and one click puts the device back the way it was.",
    integrity: "Rooted emulators can't pass Play Integrity. Jidr says so wherever it matters.",
  },
  features: {
    title: "Everything you test on, nothing to pay",
    items: [
      ["key", "One-click root", "None, Shell root or Full root with Magisk. Jidr patches, reboots and verifies, then shows the badge."],
      ["pin", "GPS and route replay", "Set a point on the map or replay a GPX/KML route at 1×, 2× or 4×."],
      ["battery", "Sensors", "Battery, rotation, network speed and delay, fingerprint, clipboard, microphone."],
      ["history", "Snapshots", "Save and restore states. Each snapshot remembers the root mode it was taken in."],
      ["bot", "Agent-native", "Every button is a CLI command and an MCP tool: screenshot, tap, type, read logs."],
      ["shell", "CLI with JSON", "Create, boot, test and destroy headless in CI. Every command has <code>--json</code>."],
    ],
  },
  compare: {
    title: "Genymotion's paid features, free",
    lead: "The things you pay for elsewhere are part of Jidr, under an open-source licence.",
    head: ["Feature", "Genymotion Desktop", "Jidr"],
    rows: ["CLI automation", "Network profiles (speed, delay)", "GPS route replay", "Quick boot", "Root toggle"],
    paid: "Paid plan",
    free: "Free",
    fine: "Plans checked 14 October 2026; see genymotion.com for current pricing. Genymotion is a trademark of its owner. Jidr is not affiliated with Genymotion.",
  },
  agents: {
    eyebrow: "MCP server",
    title: "Let an agent drive. Watch it work.",
    lead: "Add Jidr to Claude or any MCP client and the agent can see and operate a device while you watch the device window. The device shows Busy · in use by agent.",
    tools: [
      ["screenshot", "PNG of the screen"],
      ["tap", "x, y on the screenshot"],
      ["type_text", "text into the focused field"],
      ["ui_dump", "the current view hierarchy"],
      ["logcat_tail", "recent log lines, filtered"],
    ],
    client: "Claude Code",
    sessionLabel: "Example agent session",
  },
  cli: {
    title: "A few lines in CI",
    leadHtml: "Create a device, root it, run your tests, throw it away. Add <code>--json</code> to any command for output a script can read.",
    terminal: "Terminal",
  },
  download: {
    system: "macOS 14+ · Apple Silicon (arm64)",
    size: (mb: string) => `${mb} MB`,
    sha: "SHA-256",
    all: "All releases and notes on GitHub",
  },
  copy: { label: "Copy", done: "Copied" },
  docs: {
    title: "Docs",
    note: "",
    items: [
      ["docs/cli.md#quick-start", "Getting started", "Install, accept the SDK license, create your first device."],
      ["docs/root.md", "Root modes", "None, Shell root, Full root, and what Play Integrity means for you."],
      ["docs/cli.md", "CLI reference", "Every command, flag and JSON shape."],
      ["docs/mcp.md", "MCP tools", "Connect Claude and other agents. Tool list and examples."],
    ],
  },
  sponsors: {
    title: "Sponsors keep Jidr free",
    lead: "Jidr has no paid tier and no accounts. Sponsorship pays for the work.",
    sponsor: "Sponsor Jidr",
    star: "Star on GitHub",
  },
  footer: {
    licences: "App: GPL-3.0; CLI and daemon: Apache-2.0",
    source: "Source on GitHub",
    copyright: "© 2026 Jidr contributors",
    downloaded: "Downloaded on your Mac",
    downloadedItems: [
      "Android Emulator and system images: Google, under the Android SDK License, accepted in the app",
      "Magisk: GPL-3.0",
    ],
    builtWith: "Built with",
    builtWithItems: [
      "Slint: GPL-3.0",
      "Lucide icons: ISC",
      "Geist, Geist Mono, Noto Kufi Arabic: OFL-1.1",
      "Map data © OpenStreetMap contributors, ODbL",
    ],
    trademarks: "Trademarks",
    trademarkItems: ["Android is a trademark of Google LLC.", "Jidr is not affiliated with Google or Genymotion."],
  },
};

export type Strings = typeof en;

const ar: Strings = {
  lang: "ar",
  dir: "rtl",
  home: "/ar/",
  ogLocale: "ar_AR",
  title: "جذر: أندرويد على جهاز ماك، بصلاحيات الجذر في دقيقة",
  description: {
    soon: "جذر (Jidr) محاكي أندرويد مجاني ومفتوح المصدر لأجهزة ماك بمعالج Apple Silicon، مع أجهزة بصلاحيات الجذر بنقرة واحدة، وأداة سطر أوامر، وخادم MCP لوكلاء الذكاء الاصطناعي. قريبًا.",
    launch: "جذر (Jidr) محاكي أندرويد مجاني ومفتوح المصدر لأجهزة ماك بمعالج Apple Silicon، مع أجهزة بصلاحيات الجذر بنقرة واحدة، وأداة سطر أوامر، وخادم MCP لوكلاء الذكاء الاصطناعي.",
  },
  ogImageAlt: "جذر Jidr: رمز جذر، جذرٌ ينمو على هيئة هاتف",
  skip: "انتقل إلى المحتوى",
  nav: {
    label: "الموقع",
    features: "الميزات",
    root: "الجذر",
    agents: "الوكلاء",
    docs: "التوثيق",
    star: "نجمة",
  },
  other: { href: "/", lang: "en", label: "English" },
  hero: {
    pill: { soon: "قريبًا · مجاني ومفتوح المصدر", launch: "مجاني ومفتوح المصدر" },
    titleHtml: "أندرويد على جهاز ماك.<br />بصلاحيات الجذر في دقيقة.",
    lede: "جذر ينشئ هواتف وأجهزة لوحية أندرويد افتراضية ويشغّلها على Apple Silicon. اختر «جذر كامل» ويُثبَّت Magisk ويُتحقَّق منه نيابةً عنك. تطبيق سطح مكتب وأداة سطر أوامر وخادم MCP.",
    star: "ضع نجمة على GitHub",
    sponsor: "ادعم جذر",
    download: "نزّله لجهاز ماك",
    github: "اعرضه على GitHub",
    meta: { soon: ["macOS 14+", "Apple Silicon"], launch: ["macOS 14+", "Apple Silicon"] },
    version: (v: string) => `الإصدار ${v}`,
    brew: "أو brew install --cask jidr",
  },
  shotLabel: "تطبيق جذر: قائمة الأجهزة، وهاتف يشغّل تطبيق توصيل، ولوحة GPS",
  root: {
    eyebrow: "الجذر بنقرة واحدة",
    title: "اختر «جذر كامل». واحصل على شارة موثَّقة.",
    lead: "يتولى جذر الترقيع الذي كنت ستجريه بيدك، ثم يتحقق من أن su يعمل فعلًا قبل أن يعرض شارة Rooted.",
    stepsHtml: [
      "تنزيل Magisk 30.7",
      "ترقيع صورة الإقلاع مع الاحتفاظ بالأصلية",
      "إعادة التشغيل (نحو 40 ثانية)",
      "تثبيت تطبيق Magisk",
      "التحقق بالأمر <code>su -c id</code>",
    ],
    step: (n: number) => `الخطوة ${n}`,
    failure: "إن تعثّرت خطوة، ترى أيّها ولماذا مع السجل، وبنقرة واحدة يعود الجهاز كما كان.",
    integrity: "المحاكيات ذات صلاحيات الجذر لا تجتاز فحص Play Integrity، وجذر ينبّهك إلى ذلك حيثما يهمّ.",
  },
  features: {
    title: "كل ما تختبر عليه، دون أن تدفع شيئًا",
    items: [
      ["key", "الجذر بنقرة واحدة", "بلا جذر، أو جذر Shell، أو جذر كامل مع Magisk. يرقّع التطبيق الجهاز ويعيد تشغيله ويتحقق منه، ثم يعرض الشارة."],
      ["pin", "GPS وإعادة تشغيل المسارات", "حدّد نقطة على الخريطة، أو أعد تشغيل مسار GPX/KML بسرعة 1× أو 2× أو 4×."],
      ["battery", "الحسّاسات", "البطارية، والتدوير، وسرعة الشبكة وتأخيرها، والبصمة، والحافظة، والميكروفون."],
      ["history", "اللقطات", "احفظ حالات الجهاز واستعدها. كل لقطة تتذكر وضع الجذر الذي التُقطت فيه."],
      ["bot", "مصمَّم للوكلاء", "كل زر هو أمر في سطر الأوامر وأداة MCP: لقطة شاشة، ونقر، وكتابة، وقراءة السجلات."],
      ["shell", "سطر أوامر بمخرجات JSON", "أنشئ الأجهزة وشغّلها واختبرها واحذفها دون واجهة في CI. كل أمر يقبل <code>--json</code>."],
    ],
  },
  compare: {
    title: "ميزات Genymotion المدفوعة، مجانًا",
    lead: "ما تدفع مقابله في مكان آخر جزءٌ من جذر، برخصة مفتوحة المصدر.",
    head: ["الميزة", "Genymotion Desktop", "جذر"],
    rows: ["أتمتة سطر الأوامر", "ملفات الشبكة (السرعة والتأخير)", "إعادة تشغيل مسارات GPS", "الإقلاع السريع", "تبديل الجذر"],
    paid: "خطة مدفوعة",
    free: "مجانًا",
    fine: "راجعنا الخطط في 14 أكتوبر 2026؛ انظر genymotion.com للأسعار الحالية. Genymotion علامة تجارية لمالكها. جذر غير تابع لـ Genymotion.",
  },
  agents: {
    eyebrow: "خادم MCP",
    title: "دع وكيلًا يقود، وراقبه وهو يعمل.",
    lead: "أضف جذر إلى Claude أو أي عميل MCP، فيرى الوكيل الجهاز ويشغّله بينما تتابع نافذة الجهاز. ويظهر على الجهاز أن الوكيل يستخدمه.",
    tools: [
      ["screenshot", "صورة PNG للشاشة"],
      ["tap", "نقرة عند x وy على اللقطة"],
      ["type_text", "نص في الحقل المحدد"],
      ["ui_dump", "شجرة الواجهة الحالية"],
      ["logcat_tail", "آخر أسطر السجل بعد التصفية"],
    ],
    client: "Claude Code",
    sessionLabel: "مثال على جلسة وكيل",
  },
  cli: {
    title: "بضعة أسطر في CI",
    leadHtml: "أنشئ جهازًا، وامنحه صلاحيات الجذر، وشغّل اختباراتك، ثم تخلّص منه. أضف <code>--json</code> إلى أي أمر لتحصل على مخرجات يقرؤها السكربت.",
    terminal: "الطرفية",
  },
  download: {
    system: "macOS 14+ · Apple Silicon (arm64)",
    size: (mb: string) => `${mb} MB`,
    sha: "SHA-256",
    all: "كل الإصدارات وملاحظاتها على GitHub",
  },
  copy: { label: "نسخ", done: "نُسخ" },
  docs: {
    title: "التوثيق",
    note: "التوثيق بالإنجليزية حاليًا.",
    items: [
      ["docs/cli.md#quick-start", "البدء", "التثبيت، وقبول رخصة SDK، وإنشاء أول جهاز."],
      ["docs/root.md", "أوضاع الجذر", "بلا جذر، وجذر Shell، وجذر كامل، وما يعنيه Play Integrity لك."],
      ["docs/cli.md", "مرجع سطر الأوامر", "كل أمر وخيار وبنية JSON."],
      ["docs/mcp.md", "أدوات MCP", "اربط Claude والوكلاء الآخرين. قائمة الأدوات وأمثلة."],
    ],
  },
  sponsors: {
    title: "الرعاة يُبقون جذر مجانيًا",
    lead: "لا خطة مدفوعة في جذر ولا حسابات. الرعاية تموّل العمل.",
    sponsor: "ادعم جذر",
    star: "ضع نجمة على GitHub",
  },
  footer: {
    licences: "التطبيق: GPL-3.0؛ سطر الأوامر والخدمة الخلفية: Apache-2.0",
    source: "المصدر على GitHub",
    copyright: "© 2026 مساهمو جذر",
    downloaded: "يُنزَّل على جهازك",
    downloadedItems: [
      "محاكي Android وصور النظام: من Google، وفق رخصة Android SDK التي تقبلها داخل التطبيق",
      "Magisk: GPL-3.0",
    ],
    builtWith: "مبني باستخدام",
    builtWithItems: [
      "Slint: GPL-3.0",
      "أيقونات Lucide: ISC",
      "Geist وGeist Mono وNoto Kufi Arabic: OFL-1.1",
      "بيانات الخريطة © مساهمو OpenStreetMap، ODbL",
    ],
    trademarks: "العلامات التجارية",
    trademarkItems: ["Android علامة تجارية لشركة Google LLC.", "جذر غير تابع لـ Google ولا لـ Genymotion."],
  },
};

const all: Record<Locale, Strings> = { en, ar };

/** The copy for `Astro.currentLocale` (English when it's unset). */
export function strings(locale: string | undefined): Strings {
  return all[(locale as Locale) ?? "en"] ?? en;
}
