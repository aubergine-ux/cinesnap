import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { Bar } from "./screens/Bar";
import "./styles.css";

// The recording bar is a second window running the same bundle.
const bar = window.location.hash === "#bar";
if (bar) document.documentElement.classList.add("bar-window");

ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode>{bar ? <Bar /> : <App />}</React.StrictMode>);
