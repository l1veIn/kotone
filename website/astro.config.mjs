import { defineConfig } from "astro/config";

// https://astro.build/config
export default defineConfig({
  site: "https://kotone-voice.pages.dev",
  i18n: {
    locales: ["zh-CN", "en"],
    defaultLocale: "zh-CN",
    routing: {
      prefixDefaultLocale: false,
      redirectToDefaultLocale: true,
    },
  },
  build: {
    assets: "_assets",
  },
});
