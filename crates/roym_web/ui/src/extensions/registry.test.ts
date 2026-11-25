import { describe, expect, it } from "vitest";
import {
  MANIFESTS,
  allManifestMessageIds,
  choiceLabelId,
  detailsSchema,
  fieldLabelId,
  labelIds,
  manifestFor,
  ONE_SHOT_REF,
  POLL_REF,
  SCHEDULING_REF,
  stepNameId,
  stepType,
  widgetFieldLabelId,
  widgetNameId,
  widgetType,
} from "./registry";

const JSON_FILES = import.meta.glob<unknown>("../../../../roym_core/extensions/*.json", {
  import: "default",
  eager: true,
});

describe("the Hub's list of extensions", () => {
  // `roym_core` checks the other direction: its tests read this module's
  // imports and compare them with the manifests it compiles in.
  it("holds every manifest file, and nothing else", () => {
    const files = Object.values(JSON_FILES);
    expect(files.length).toBeGreaterThan(0);
    expect([...MANIFESTS].sort((a, b) => a.id.localeCompare(b.id))).toEqual(
      [...files].sort((a, b) => (a as { id: string }).id.localeCompare((b as { id: string }).id)),
    );
  });

  it("finds a workflow manifest by id and version", () => {
    expect(manifestFor({ id: "scheduling", version: 1 })?.id).toBe("scheduling");
    expect(manifestFor({ id: "scheduling", version: 2 })).toBeUndefined();
    expect(manifestFor({ id: "future", version: 1 })).toBeUndefined();
    expect(manifestFor(SCHEDULING_REF)).toBeDefined();
    expect(manifestFor(ONE_SHOT_REF)).toBeDefined();
  });

  it("finds a schema and a step type", () => {
    const ext = { id: "scheduling", version: 1 };
    expect(detailsSchema(ext, "scheduling/slot@1")?.fields.map((f) => f.name)).toEqual([
      "listing_id",
      "slot_id",
    ]);
    expect(detailsSchema(ext, "scheduling/other@1")).toBeUndefined();
    expect(stepType(ext, "scheduling/started")?.by).toBe("giver");
  });
});

describe("the message ids", () => {
  it("are derived as the node derives them", () => {
    expect(fieldLabelId("scheduling", "scheduling/slot@1", "listing_id")).toBe(
      "ext-scheduling-slot-listing-id",
    );
    expect(stepNameId("scheduling", "scheduling/started")).toBe("ext-scheduling-step-started");
    expect(choiceLabelId("a-b", "a-b/some_form@2", "kind_of", "big-one")).toBe(
      "ext-a-b-some-form-kind-of-big-one",
    );
    expect(allManifestMessageIds()).toEqual([
      "ext-scheduling-slot-listing-id",
      "ext-scheduling-slot-slot-id",
      "ext-scheduling-step-started",
      "ext-poll-widget-single",
      "ext-poll-widget-single-option",
    ]);
  });

  it("derives widget message ids and finds widget type", () => {
    expect(widgetNameId("poll", "poll/single")).toBe("ext-poll-widget-single");
    expect(widgetFieldLabelId("poll", "poll/single", "option")).toBe(
      "ext-poll-widget-single-option",
    );
    expect(widgetType(POLL_REF, "poll/single")).toBeDefined();
    expect(widgetType({ id: "scheduling", version: 1 }, "poll/single")).toBeUndefined();
  });

  it("are keyed by field name, and by field and choice, for a schema", () => {
    const ids = labelIds({ id: "scheduling", version: 1 }, "scheduling/slot@1");
    expect([...ids.entries()]).toEqual([
      ["listing_id", "ext-scheduling-slot-listing-id"],
      ["slot_id", "ext-scheduling-slot-slot-id"],
    ]);
    expect(labelIds({ id: "future", version: 1 }, "future/x@1").size).toBe(0);
  });
});
