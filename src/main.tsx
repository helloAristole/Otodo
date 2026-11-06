import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { IS_TAURI } from "./api";
import "./styles.css";

// 浏览器预览时给页面一个背景，让毛玻璃效果可见（Tauri 下保持透明）
if (!IS_TAURI) document.body.classList.add("browser-mode");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
