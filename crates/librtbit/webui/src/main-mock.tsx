// Mock entry point for testing UI with large number of torrents
// Run with: npm run dev:mock

import { StrictMode } from "react";
import ReactDOM from "react-dom/client";
import { RtbitWebUI } from "./rtbit-web";
import { APIContext } from "./context";
import { RssAPI } from "./http-api";
import { MockAPI, MockRssAPI } from "./mock-api";
import "./globals.css";

Object.assign(RssAPI, MockRssAPI);

const RootWithMockAPI = () => {
  return (
    <APIContext.Provider value={MockAPI}>
      <RtbitWebUI title="RustTorrent Demo" version="v0.1.0-beta.3" />
    </APIContext.Provider>
  );
};

ReactDOM.createRoot(document.getElementById("app") as HTMLInputElement).render(
  <StrictMode>
    <RootWithMockAPI />
  </StrictMode>,
);
