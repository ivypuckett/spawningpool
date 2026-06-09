import { describe, it, expect, vi, beforeEach } from "vitest";
vi.mock("../lib/api", () => ({ showEntity: vi.fn() }));
import { render, screen } from "@testing-library/svelte";
import { showEntity } from "../lib/api";
import { selection } from "../lib/stores";
import EntityDetail from "./EntityDetail.svelte";

describe("EntityDetail", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    selection.set(null);
  });

  it("shows a placeholder when nothing is selected", () => {
    render(EntityDetail);
    expect(screen.getByText(/select an item/i)).toBeInTheDocument();
  });

  it("renders the fetched definition for the selection", async () => {
    vi.mocked(showEntity).mockResolvedValue({
      name: "anthropic",
      base_url: "https://api.anthropic.com",
    });
    selection.set({ kind: "provider", name: "anthropic" });
    render(EntityDetail);
    // the JSON should appear (await async effect/fetch)
    expect(await screen.findByText(/api\.anthropic\.com/)).toBeInTheDocument();
    expect(showEntity).toHaveBeenCalledWith("provider", "anthropic");
  });

  it("renders an error when the fetch fails", async () => {
    vi.mocked(showEntity).mockRejectedValue("no such provider ghost");
    selection.set({ kind: "provider", name: "ghost" });
    render(EntityDetail);
    expect(await screen.findByText(/no such provider ghost/)).toBeInTheDocument();
  });
});
