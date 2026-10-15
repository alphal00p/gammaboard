import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { fetchSession, login as loginRequest, logout as logoutRequest } from "../services/api";

const AuthContext = createContext(null);

export const AuthProvider = ({ children }) => {
  const [authEnabled, setAuthEnabled] = useState(true);
  const [authenticated, setAuthenticated] = useState(false);
  const [allowLocalNodeSpawn, setAllowLocalNodeSpawn] = useState(true);
  const [sessionError, setSessionError] = useState(null);
  const [ready, setReady] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);

  const refreshSession = useCallback(async () => {
    setReady(false);
    try {
      const response = await fetchSession();
      setAuthEnabled(response?.auth_enabled !== false);
      setAuthenticated(response?.authenticated === true && (response?.auth_enabled !== false || sessionStorage.getItem("gammaboard.readOnly") !== "true"));
      setAllowLocalNodeSpawn(response?.allow_local_node_spawn !== false);
      setSessionError(null);
      setError(null);
    } catch (err) {
      setAuthenticated(false);
      setAllowLocalNodeSpawn(true);
      setSessionError(err?.message || "Failed to check browser access");
      setError(err?.status === 401 ? null : err?.message || "Failed to load auth session");
    } finally {
      setReady(true);
    }
  }, []);

  useEffect(() => {
    refreshSession();
  }, [refreshSession]);

  const requestLogin = useCallback(() => {
    setError(null);
    if (!authEnabled) {
      sessionStorage.removeItem("gammaboard.readOnly");
      setAuthenticated(true);
      return;
    }
    setDialogOpen(true);
  }, [authEnabled]);

  const login = useCallback(async (password) => {
    setBusy(true);
    try {
      await loginRequest(password);
      setAuthenticated(true);
      setDialogOpen(false);
      setError(null);
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
      if (authEnabled) await logoutRequest();
      else sessionStorage.setItem("gammaboard.readOnly", "true");
      setAuthenticated(false);
      setError(null);
    } finally {
      setBusy(false);
    }
  }, [authEnabled]);

  const value = useMemo(
    () => ({
      authenticated,
      authEnabled,
      allowLocalNodeSpawn,
      ready,
      sessionError,
      busy,
      error,
      dialogOpen,
      setDialogOpen,
      login,
      logout,
      requestLogin,
      refreshSession,
    }),
    [
      authenticated,
      authEnabled,
      allowLocalNodeSpawn,
      ready,
      sessionError,
      busy,
      error,
      dialogOpen,
      login,
      logout,
      requestLogin,
      refreshSession,
    ],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
};

export const useAuth = () => {
  const value = useContext(AuthContext);
  if (!value) throw new Error("useAuth must be used within AuthProvider");
  return value;
};
