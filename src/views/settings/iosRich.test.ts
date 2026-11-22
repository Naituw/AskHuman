import { describe, expect, it } from "vitest";
import { parseRich } from "./iosRich";

describe("parseRich", () => {
  it("leaves plain text as one piece", () => {
    expect(parseRich("Just words.")).toEqual([{ kind: "text", text: "Just words." }]);
    expect(parseRich("")).toEqual([]);
  });

  it("reads bold, code, links and chips in the middle of a sentence", () => {
    expect(parseRich("[Open it ↗](console), create a **Serverless** deployment: topic [[acl]], path `/mqtt`.")).toEqual([
      { kind: "link", text: "Open it ↗", id: "console" },
      { kind: "text", text: ", create a " },
      { kind: "bold", text: "Serverless" },
      { kind: "text", text: " deployment: topic " },
      { kind: "chip", id: "acl" },
      { kind: "text", text: ", path " },
      { kind: "code", text: "/mqtt" },
      { kind: "text", text: "." },
    ]);
  });

  it("keeps the text around the markup, spaces included", () => {
    expect(parseRich("a **b** c")).toEqual([
      { kind: "text", text: "a " },
      { kind: "bold", text: "b" },
      { kind: "text", text: " c" },
    ]);
  });

  it("does not mistake a chip for a link, nor brackets without a target for either", () => {
    expect(parseRich("[[all]]")).toEqual([{ kind: "chip", id: "all" }]);
    expect(parseRich("[not a link] and (not a target)")).toEqual([
      { kind: "text", text: "[not a link] and (not a target)" },
    ]);
  });

  it("never turns text into HTML: markup characters stay data", () => {
    expect(parseRich("<b>x</b> & **y**")).toEqual([
      { kind: "text", text: "<b>x</b> & " },
      { kind: "bold", text: "y" },
    ]);
  });
});
