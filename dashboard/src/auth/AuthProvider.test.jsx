import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { AuthProvider, useAuth } from "./AuthProvider";
import { fetchSession, login, logout } from "../services/api";

vi.mock("../services/api", () => ({ fetchSession: vi.fn(), login: vi.fn(), logout: vi.fn() }));

function Controls() {
  const auth = useAuth();
  return <>
    <span>{auth.ready ? (auth.authenticated ? "controls enabled" : "read-only") : "loading"}</span>
    <span>{auth.dialogOpen ? "password dialog" : "no dialog"}</span>
    <button onClick={auth.logout}>disable</button>
    <button onClick={auth.requestLogin}>enable</button>
  </>;
}

beforeEach(() => { vi.clearAllMocks(); sessionStorage.clear(); });

test("passwordless controls toggle locally, persist across reload, and never ask for a password", async () => {
  fetchSession.mockResolvedValue({ authenticated: true, auth_enabled: false });
  const view = render(<AuthProvider><Controls /></AuthProvider>);
  await screen.findByText("controls enabled");
  fireEvent.click(screen.getByText("disable"));
  await screen.findByText("read-only");
  expect(logout).not.toHaveBeenCalled();
  view.unmount();
  render(<AuthProvider><Controls /></AuthProvider>);
  await screen.findByText("read-only");
  fireEvent.click(screen.getByText("enable"));
  await screen.findByText("controls enabled");
  expect(screen.getByText("no dialog")).toBeInTheDocument();
  expect(login).not.toHaveBeenCalled();
});

test("password-protected access still uses the login dialog and server logout", async () => {
  fetchSession.mockResolvedValue({ authenticated: true, auth_enabled: true });
  logout.mockResolvedValue({ authenticated: false });
  render(<AuthProvider><Controls /></AuthProvider>);
  await screen.findByText("controls enabled");
  fireEvent.click(screen.getByText("disable"));
  await waitFor(() => expect(logout).toHaveBeenCalledOnce());
  await screen.findByText("read-only");
  fireEvent.click(screen.getByText("enable"));
  await screen.findByText("password dialog");
});
