import React from "react";
import ReactDOM from "react-dom/client";
import { Theme } from "@radix-ui/themes";
import "@radix-ui/themes/styles.css";
import App from "./App";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Theme className="app-theme" accentColor="blue" grayColor="slate" radius="medium" appearance="light" hasBackground={false}>
      <App />
    </Theme>
  </React.StrictMode>,
);
