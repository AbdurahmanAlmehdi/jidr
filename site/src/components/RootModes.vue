<script setup lang="ts">
import { computed, ref } from "vue";

interface Mode {
  id: "none" | "adb" | "magisk";
  label: string;
  summary: string;
  steps: string[];
  verify: string;
  result: string;
  images: string;
}

const modes: Mode[] = [
  {
    id: "none",
    label: "None",
    summary: "A stock device, exactly as the system image ships.",
    steps: ["Boot the image as-is"],
    verify: "adb shell id",
    result: "uid=2000(shell)",
    images: "Every image",
  },
  {
    id: "adb",
    label: "Shell root",
    summary: "Root for adb only. Apps on the device stay unrooted.",
    steps: ["Boot", "Restart adbd as root", "Verify"],
    verify: "adb shell id",
    result: "uid=0(root)",
    images: "Google APIs and AOSP images (Play Store images don't allow it)",
  },
  {
    id: "magisk",
    label: "Full root",
    summary:
      "Magisk, set up for you. Apps ask for root through Magisk's normal prompt; nothing is granted silently.",
    steps: [
      "Download the official Magisk release and check its SHA-256",
      "Patch a private copy of the ramdisk (your SDK stays untouched)",
      "Cold boot, install Magisk, reboot once",
      "Verify",
    ],
    verify: "adb shell su -c id",
    result: "uid=0(root)",
    images: "Google APIs and AOSP images; Play Store images in beta",
  },
];

const selected = ref<Mode["id"]>("magisk");
const mode = computed(() => modes.find((m) => m.id === selected.value) ?? modes[2]);
</script>

<template>
  <div class="root-modes">
    <fieldset>
      <legend class="visually-hidden">Root mode</legend>
      <label v-for="m in modes" :key="m.id" class="card" :class="{ active: selected === m.id }">
        <input v-model="selected" type="radio" name="root-mode" :value="m.id" />
        <span class="card-label">{{ m.label }}</span>
        <span class="card-summary">{{ m.summary }}</span>
      </label>
    </fieldset>

    <div class="detail" aria-live="polite">
      <h3>{{ mode.label }}: what Jidr does</h3>
      <ol>
        <li v-for="step in mode.steps" :key="step">{{ step }}</li>
      </ol>
      <dl>
        <div>
          <dt>Check</dt>
          <dd><code>{{ mode.verify }}</code> → <code>{{ mode.result }}</code></dd>
        </div>
        <div>
          <dt>Images</dt>
          <dd>{{ mode.images }}</dd>
        </div>
      </dl>
    </div>
  </div>
</template>

<style scoped>
.root-modes {
  display: grid;
  gap: var(--jidr-space-6);
  margin-top: var(--jidr-space-8);
}
@media (min-width: 900px) {
  .root-modes {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  }
}
fieldset {
  display: grid;
  gap: var(--jidr-space-3);
  margin: 0;
  padding: 0;
  border: 0;
  min-width: 0;
}
.card {
  display: grid;
  grid-template-columns: auto 1fr;
  column-gap: var(--jidr-space-3);
  row-gap: var(--jidr-space-1);
  padding: var(--jidr-space-4);
  border: var(--jidr-border-width) solid var(--jidr-color-border-default);
  border-radius: var(--jidr-radius-md);
  background: var(--jidr-color-surface-raised);
  cursor: pointer;
}
.card.active {
  border-color: var(--jidr-color-accent);
  box-shadow: inset 0 0 0 1px var(--jidr-color-accent);
}
.card input {
  grid-row: span 2;
  margin: 0.2rem 0 0;
  accent-color: var(--jidr-color-accent);
  width: 18px;
  height: 18px;
}
.card-label {
  font-weight: 650;
}
.card-summary {
  color: var(--jidr-color-text-secondary);
  font-size: var(--jidr-text-label);
}
.detail {
  padding: var(--jidr-space-6);
  border-radius: var(--jidr-radius-md);
  background: var(--jidr-color-surface-sunken);
  border: var(--jidr-border-width) solid var(--jidr-color-border-subtle);
  min-width: 0;
}
h3 {
  font-size: var(--jidr-text-title);
}
ol {
  margin: var(--jidr-space-4) 0 0;
  padding-inline-start: var(--jidr-space-6);
  color: var(--jidr-color-text-secondary);
}
li + li {
  margin-top: var(--jidr-space-1);
}
dl {
  display: grid;
  gap: var(--jidr-space-3);
  margin: var(--jidr-space-6) 0 0;
  font-size: var(--jidr-text-label);
}
dt {
  font-weight: 600;
  color: var(--jidr-color-text-tertiary);
}
dd {
  margin: 0;
  overflow-wrap: anywhere;
}
code {
  font-size: var(--jidr-text-caption);
  color: var(--jidr-color-text-code);
}
</style>
