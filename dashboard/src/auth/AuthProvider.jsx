import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { fetchSession, login as loginRequest, logout as logoutRequest } from "../services/api";

const AuthContext = createContext(null);

export const AuthProvider = ({ children }) => {
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
      setAuthenticated(response?.authenticated === true);
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
    setDialogOpen(true);
  }, []);

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
      await logoutRequest();
      setAuthenticated(false);
      setError(null);
    } finally {
      setBusy(false);
    }
  }, []);

  const value = useMemo(
    () => ({
      authenticated,
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
