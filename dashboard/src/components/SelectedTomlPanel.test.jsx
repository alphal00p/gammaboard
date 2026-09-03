import { render, screen } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import SelectedTomlPanel from "./SelectedTomlPanel";

describe("SelectedTomlPanel", () => {
  test.each([
    ["Run", "demo run", 'name = "demo"'],
    ["Task", "sample", 'name = "sample"'],
  ])("renders the selected %s TOML", (kind, name, toml) => {
    render(<SelectedTomlPanel kind={kind} name={name} toml={toml} />);

    expect(screen.getByText(`Selected ${kind} TOML`)).toBeInTheDocument();
    expect(screen.getByText(name)).toBeInTheDocument();
    expect(screen.getByText(toml)).toBeInTheDocument();
  });
});
