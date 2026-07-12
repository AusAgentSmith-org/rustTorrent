import { StrictMode, useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import { RtbitWebUI } from "./rtbit-web";
import { customSetInterval } from "./helper/customSetInterval";
import { APIContext } from "./context";
import { API, AuthAPI } from "./http-api";
import { useAuthStore } from "./stores/authStore";
import { LoginPage } from "./components/auth/LoginPage";
import { SetupPage } from "./components/auth/SetupPage";
import { Spinner } from "./components/Spinner";
import { useStatsStore } from "./stores/statsStore";
import "./globals.css";

const AuthGate = ({ children }: { children: React.ReactNode }) => {
  const authState = useAuthStore((s) => s.state);
  const setState = useAuthStore((s) => s.setState);
  const accessToken = useAuthStore((s) => s.accessToken);

  useEffect(() => {
    const checkAuth = async () => {
      try {
        const status = await AuthAPI.getStatus();

        if (status.setup_required) {
          setState("setup_required");
          return;
        }

        if (!status.auth_enabled) {
          setState("no_auth");
          return;
        }

        // Auth is enabled — check if we have a valid token
        if (accessToken) {
          setState("authenticated");
        } else {
          setState("login_required");
        }
      } catch {
        // Can't reach server or /auth/status not available — assume no auth
        setState("no_auth");
      }
    };
    checkAuth();
  }, []);

  // Re-check when accessToken changes (e.g., after login/setup)
  useEffect(() => {
    if (authState === "loading") return;
    if (accessToken && authState !== "authenticated") {
      setState("authenticated");
    }
  }, [accessToken]);

  if (authState === "loading") {
    return (
      <div className="bg-surface h-dvh flex items-center justify-center">
        <Spinner />
      </div>
    );
  }

  if (authState === "setup_required") {
    return <SetupPage />;
  }

  if (authState === "login_required") {
    return <LoginPage />;
  }

  return <>{children}</>;
};

const RootWithVersion = () => {
  const [version, setVersion] = useState<string>("");
  const downloadSpeed = useStatsStore(
    (s) => s.stats.download_speed.human_readable,
  );

  useEffect(() => {
    const refreshVersion = () =>
      API.getVersion().then(
        (version) => {
          setVersion((prev) => {
            if (prev == version) {
              return prev;
            }
            return version;
          });
          return 60000;
        },
        (e) => {
          return 1000;
        },
      );
    return customSetInterval(refreshVersion, 0);
  }, []);

  useEffect(() => {
    document.title = `Rust Torrent - ↓ ${downloadSpeed}`;
  }, [downloadSpeed]);

  return (
    <APIContext.Provider value={API}>
      <AuthGate>
        <RtbitWebUI title="rtbit" version={version} />
      </AuthGate>
    </APIContext.Provider>
  );
};

ReactDOM.createRoot(document.getElementById("app") as HTMLInputElement).render(
  <StrictMode>
    <RootWithVersion />
  </StrictMode>,
);
