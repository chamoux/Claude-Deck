import { createApp } from "vue";
import App from "./App.vue";
import { tauriTransport } from "./transport";
import { createDeck, deckKey } from "./useDeck";
import "@fontsource-variable/ibm-plex-sans";
import "@fontsource-variable/jetbrains-mono";
import "./style.css";

async function boot(): Promise<void> {
  let transport = tauriTransport();
  let afterStart: ((deck: ReturnType<typeof createDeck>) => void) | undefined;
  if (import.meta.env.DEV && new URLSearchParams(location.search).has("demo")) {
    const { createDemoTransport } = await import("./demo");
    const demo = createDemoTransport();
    const scene = new URLSearchParams(location.search).get("scene");
    transport = demo;
    afterStart = (d) => void demo.runScene(d, scene);
  }
  const deck = createDeck(transport);
  const app = createApp(App);
  app.provide(deckKey, deck);
  deck.start();
  app.mount("#app");
  afterStart?.(deck);
}
void boot();
