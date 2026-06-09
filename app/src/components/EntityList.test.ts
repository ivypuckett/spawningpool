import { describe, it, expect, beforeEach } from "vitest";
import { render, fireEvent, screen } from "@testing-library/svelte";
import { get } from "svelte/store";
import { selection } from "../lib/stores";
import EntityList from "./EntityList.svelte";

describe("EntityList", () => {
  beforeEach(() => selection.set(null));

  it("renders each name", () => {
    render(EntityList, { props: { kind: "specialist", names: ["summarizer", "netop"] } });
    expect(screen.getByText("summarizer")).toBeInTheDocument();
    expect(screen.getByText("netop")).toBeInTheDocument();
  });

  it("clicking a name sets the selection store", async () => {
    render(EntityList, { props: { kind: "specialist", names: ["summarizer", "netop"] } });
    await fireEvent.click(screen.getByText("netop"));
    expect(get(selection)).toEqual({ kind: "specialist", name: "netop" });
    expect(screen.getByText("netop").closest("button")).toHaveAttribute("aria-current", "true");
  });

  it("filters names case-insensitively", async () => {
    render(EntityList, { props: { kind: "model", names: ["claude-opus", "qwen"] } });
    const input = screen.getByRole("textbox");
    await fireEvent.input(input, { target: { value: "QWEN" } });
    expect(screen.queryByText("claude-opus")).not.toBeInTheDocument();
    expect(screen.getByText("qwen")).toBeInTheDocument();
  });

  it("shows an empty state when there are no names", () => {
    render(EntityList, { props: { kind: "tool", names: [] } });
    expect(screen.getByText(/no tools yet/i)).toBeInTheDocument();
  });
});
