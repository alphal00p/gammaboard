import { useState } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test } from "vitest";
import { EditorView } from "@codemirror/view";
import TomlEditor from "./TomlEditor";
import { DashboardTheme, ThemeToggle } from "../../theme";

beforeEach(() => window.localStorage.clear());

test("keeps exact TOML, selection and editor instance across theme changes; restores the theme", async () => {
  const original = '# comment\nsamples = "$(samples:10000)"\nconfig = {\n  seed = 1,\n}\n';
  function Form() {
    const [value, setValue] = useState(original);
    return <DashboardTheme><ThemeToggle /><TomlEditor label="Run TOML" value={value} onChange={setValue} /></DashboardTheme>;
  }
  const first = render(<Form />);
  const input = await screen.findByRole("textbox", { name: "Run TOML" });
  const view = EditorView.findFromDOM(input);
  expect(view.state.doc.toString()).toBe(original);
  expect(first.container.querySelector(".cm-foldGutter")).toBeNull();
  expect(first.container.querySelector(".cm-line span")).not.toBeNull();
  act(() => view.dispatch({ changes: { from: original.length, insert: '# unfinished "' }, selection: { anchor: 4 } }));
  fireEvent.click(screen.getByRole("button", { name: "Switch to dark theme" }));
  await waitFor(() => expect(view.state.facet(EditorView.darkTheme)).toBe(true));
  expect(EditorView.findFromDOM(input)).toBe(view);
  expect(view.state.doc.toString()).toBe(original + '# unfinished "');
  expect(view.state.selection.main.anchor).toBe(4);
  expect(localStorage.getItem("gammaboard.theme")).toBe("dark");
  first.unmount();
  const second = render(<Form />);
  expect(screen.getByRole("button", { name: "Switch to light theme" })).toBeInTheDocument();
  await waitFor(() => expect(second.container.querySelector(".cm-theme-dark")).not.toBeNull());
  fireEvent.click(screen.getByRole("button", { name: "Switch to light theme" }));
  await waitFor(() => expect(second.container.querySelector(".cm-theme-light")).not.toBeNull());
});

test("loading disables input and an externally selected template replaces the document", async () => {
  const props = { label: "Task TOML", value: 'name = "first"', onChange: () => {} };
  const { rerender } = render(<TomlEditor {...props} />);
  const input = await screen.findByRole("textbox", { name: "Task TOML" });
  rerender(<TomlEditor {...props} disabled />);
  expect(input).toHaveAttribute("contenteditable", "false");
  expect(input).toHaveAttribute("aria-disabled", "true");
  rerender(<TomlEditor {...props} value={'name = "second"\n# intact'} />);
  expect(EditorView.findFromDOM(input).state.doc.toString()).toBe('name = "second"\n# intact');
  expect(input).toHaveAttribute("contenteditable", "true");
});
