import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { fetchSession, login as loginRequest, logout as logoutRequest } from "../services/api";

const AuthContext = createContext(null);

export const AuthProvider = ({ children }) => {
  const [authenticated, setAuthenticated] = useState(false);
  const [ready, setReady] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const pendingActionRef = useRef(null);

  const refreshSession = useCallback(async () => {
    try {
      const response = await fetchSession();
      setAuthenticated(response?.authenticated === true);
      setError(null);
    } catch (err) {
      setAuthenticated(false);
      setError(err?.status === 401 ? null : err?.message || "Failed to load auth session");
    } finally {
      setReady(true);
    }
  }, []);

  useEffect(() => {
    refreshSession();
  }, [refreshSession]);

  const requestLogin = useCallback((action = null) => {
    pendingActionRef.current = typeof action === "function" ? action : null;
    setError(null);
    setDialogOpen(true);
  }, []);

  const login = useCallback(async (password) => {
    setBusy(true);
    try {
      await loginRequest(password);
      setAuthenticated(true);
      setDialogOpen(false);
      setError(null);
      const action = pendingActionRef.current;
      pendingActionRef.current = null;
      await action?.();
      return true;
    } catch (err) {
      setAuthenticated(false);
      setError(err?.message || "Login failed");
      return false;
    } finally {
      setBusy(false);
    }
  }, []);

  const logout = useCallback(async () => {
    setBusy(true);
    try {
      await logoutRequest();
      setAuthenticated(false);
      setError(null);
    } finally {
      setBusy(false);
    }
  }, []);

  const requireAuth = useCallback(
    async (action) => {
      if (authenticated) {
        try {
          await action?.();
        } catch (err) {
          if (err?.status === 401) {
            setAuthenticated(false);
            requestLogin(action);
            return;
          }
          throw err;
        }
        return;
      }
      requestLogin(action);
    },
    [authenticated, requestLogin],
  );

  const value = useMemo(
    () => ({
      authenticated,
      ready,
      busy,
      error,
      dialogOpen,
      setDialogOpen,
      login,
      logout,
      requestLogin,
      requireAuth,
      refreshSession,
    }),
    [authenticated, ready, busy, error, dialogOpen, login, logout, requestLogin, requireAuth, refreshSession],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
};

export const useAuth = () => {
  const value = useContext(AuthContext);
  if (!value) throw new Error("useAuth must be used within AuthProvider");
  return value;
};
