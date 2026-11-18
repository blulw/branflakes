import { describe, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
import { formatBytes, isBranPath, safeFileName } from "./bran";

describe("bran helpers", () => {
  it("recognises .bran paths case-insensitively", () => {
    expect(isBranPath("C:\\x\\Road trip.BRAN")).toBe(true);
    expect(isBranPath("C:\\x\\song.mp3")).toBe(false);
    expect(isBranPath("C:\\x\\bran")).toBe(false);
  });
  it("makes Windows-safe file names", () => {
    expect(safeFileName('AC/DC: "Best" <hits>?')).toBe("AC_DC_ _Best_ _hits__");
    expect(safeFileName("...")).toBe("playlist");
    expect(safeFileName("Mix. ")).toBe("Mix");
  });
  it("formats sizes", () => {
    expect(formatBytes(500)).toBe("500 B");
    expect(formatBytes(2048)).toBe("2 KB");
    expect(formatBytes(5 * 1024 ** 2)).toBe("5.0 MB");
  });
});
