import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import "./styles/app.css";

// Apply transparent HUD mode synchronously before first paint so overlay windows never flash
// an opaque dark background frame.
if (window.location.hash.startsWith("#hud-")) {
  document.documentElement.classList.add("hud-mode");
  document.body.classList.add("hud-mode");
}

const root = document.getElementById("root");
if (!root) throw new Error("#root is missing from index.html");

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
