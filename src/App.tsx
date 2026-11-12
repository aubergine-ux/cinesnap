import { useEffect } from "react";
import { api } from "./lib/api";
import { useStore } from "./lib/store";
import { Editor } from "./screens/Editor";
import { Home } from "./screens/Home";
import { Recording } from "./screens/Recording";

export function App() {
  const screen = useStore((s) => s.screen);
  const hasProject = useStore((s) => s.project !== null);
  const error = useStore((s) => s.error);
  const setError = useStore((s) => s.setError);

  useEffect(() => {
    void useStore.getState().init();
  }, []);

  useEffect(() => {
    if (!error) return;
    // Also in the backend log, for when the app is started from a terminal.
    api.clientLog(`error: ${error}`);
    const t = setTimeout(() => setError(null), 8000);
    return () => clearTimeout(t);
  }, [error, setError]);

  return (
    <>
      {screen === "editor" && hasProject ? <Editor /> : screen === "recording" ? <Recording /> : <Home />}
      {error && (
        <div className="toast" role="alert" onClick={() => setError(null)}>
          {error}
        </div>
      )}
    </>
  );
}
