import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app/App";
import { Editor } from "./app/editor";
import type { EngineClient } from "./engine/client";
import { TauriEngine, inDesktopApp } from "./engine/tauri";
import { setLanguage, t } from "./i18n";
import "./styles.css";

const root = createRoot(document.getElementById("root") as HTMLElement);

function start(engine: EngineClient) {
  const editor = new Editor(engine, (question) => window.confirm(question));
  root.render(
    <StrictMode>
      <App editor={editor} />
    </StrictMode>,
  );
}

if (inDesktopApp()) {
  start(new TauriEngine());
} else if (import.meta.env.DEV) {
  // In a plain browser, development builds show a sample project to work on the interface.
  document.body.classList.add("browser");
  void import("./engine/demo").then(({ demoEngine }) => start(demoEngine()));
} else {
  setLanguage(navigator.language.slice(0, 2));
  root.render(
    <div className="start">
      <div className="card">
        <h1>FossCut</h1>
        <p className="muted">{t("app.desktopOnly")}</p>
      </div>
    </div>,
  );
}
