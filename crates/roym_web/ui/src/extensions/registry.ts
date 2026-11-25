// The Roym extensions that ship with this build. The manifests are the same
// JSON files that `roym_core` compiles in, so this Hub and the node it talks
// to always agree on which extensions are known.

import oneShot from "../../../../roym_core/extensions/one-shot.json";
import poll from "../../../../roym_core/extensions/poll.json";
import scheduling from "../../../../roym_core/extensions/scheduling.json";

export type FieldType =
  | "text"
  | "long-text"
  | "code"
  | "number"
  | "money"
  | "date-time"
  | "duration"
  | "choice";

export interface FieldDef {
  name: string;
  type: FieldType;
  required?: boolean;
  max_len?: number;
  min?: number;
  max?: number;
  choices?: string[];
}

export interface SchemaDef {
  schema: string;
  fields: FieldDef[];
  checks?: string[];
}

export interface StepDef {
  type: string;
  by: "giver" | "receiver" | "either";
  fields: FieldDef[];
}

export interface WidgetFieldRuleDef {
  name: string;
  type: FieldType;
  required?: boolean;
  max_len?: number;
  min?: number;
  max?: number;
  min_choices?: number;
  max_choices?: number;
}

export interface WidgetTypeDef {
  type: string;
  semantic: string;
  fields: WidgetFieldRuleDef[];
}

export interface Manifest {
  id: string;
  version: number;
  kind: "workflow" | "insight" | "chat";
  resource_kinds?: string[];
  hooks?: string[];
  details?: SchemaDef[];
  steps?: StepDef[];
  reads?: string[];
  widgets?: WidgetTypeDef[];
}

export interface ExtensionRef {
  id: string;
  version: number;
}

export const SCHEDULING_EXTENSION = "scheduling";
export const SCHEDULING_SLOT_SCHEMA = "scheduling/slot@1";
export const ONE_SHOT_EXTENSION = "one-shot";
export const POLL_EXTENSION = "poll";

/// What a listing writes to name each shipped extension. A test checks that
/// `manifestFor` finds both.
export const SCHEDULING_REF: ExtensionRef = { id: SCHEDULING_EXTENSION, version: 1 };
export const ONE_SHOT_REF: ExtensionRef = { id: ONE_SHOT_EXTENSION, version: 1 };
export const POLL_REF: ExtensionRef = { id: POLL_EXTENSION, version: 1 };

/// In the order of `MANIFESTS` in `roym_core/src/extension.rs`.
export const MANIFESTS: readonly Manifest[] = [scheduling, oneShot, poll] as unknown as Manifest[];

/// A `workflow` manifest only. An `insight` id is unknown here, as it is to
/// the node.
export function manifestFor(ext: ExtensionRef): Manifest | undefined {
  return MANIFESTS.find((m) => m.kind === "workflow" && m.id === ext.id && m.version === ext.version);
}

export function detailsSchema(ext: ExtensionRef, schema: string): SchemaDef | undefined {
  return manifestFor(ext)?.details?.find((s) => s.schema === schema);
}

export function stepType(ext: ExtensionRef, type: string): StepDef | undefined {
  return manifestFor(ext)?.steps?.find((s) => s.type === type);
}

export function widgetType(ext: ExtensionRef, type: string): WidgetTypeDef | undefined {
  return MANIFESTS.find((m) => m.kind === "chat" && m.id === ext.id && m.version === ext.version)?.widgets?.find((w) => w.type === type);
}

// Message ids are derived from codes, as the node derives them. The schema
// version is not part of an id.

function dashed(s: string): string {
  return s.replaceAll("_", "-");
}

function afterPrefix(extId: string, name: string): string {
  const prefix = `${extId}/`;
  return name.startsWith(prefix) ? name.slice(prefix.length) : name;
}

function schemaShort(extId: string, schema: string): string {
  return afterPrefix(extId, schema).split("@")[0];
}

export function fieldLabelId(extId: string, schema: string, field: string): string {
  return `ext-${dashed(extId)}-${dashed(schemaShort(extId, schema))}-${dashed(field)}`;
}

export function choiceLabelId(extId: string, schema: string, field: string, choice: string): string {
  return `${fieldLabelId(extId, schema, field)}-${dashed(choice)}`;
}

export function stepNameId(extId: string, type: string): string {
  return `ext-${dashed(extId)}-step-${dashed(afterPrefix(extId, type))}`;
}

export function widgetNameId(extId: string, type: string): string {
  return `ext-${dashed(extId)}-widget-${dashed(afterPrefix(extId, type))}`;
}

export function widgetFieldLabelId(extId: string, type: string, field: string): string {
  return `ext-${dashed(extId)}-widget-${dashed(afterPrefix(extId, type))}-${dashed(field)}`;
}

/// The message id of each field of a schema, keyed by the field name, and of
/// each choice, keyed `field:choice`. Read it through `codeWord`, so a field
/// from another build is drawn under its own name.
export function labelIds(ext: ExtensionRef, schema: string): Map<string, string> {
  const ids = new Map<string, string>();
  const def = detailsSchema(ext, schema);
  for (const field of def?.fields ?? []) {
    ids.set(field.name, fieldLabelId(ext.id, schema, field.name));
    for (const choice of field.choices ?? []) {
      ids.set(`${field.name}:${choice}`, choiceLabelId(ext.id, schema, field.name, choice));
    }
  }
  return ids;
}

/// Every message id that one manifest implies, in manifest order.
export function manifestMessageIds(manifest: Manifest): string[] {
  const ids: string[] = [];
  for (const schema of manifest.details ?? []) {
    for (const field of schema.fields) {
      ids.push(fieldLabelId(manifest.id, schema.schema, field.name));
      for (const choice of field.choices ?? []) {
        ids.push(choiceLabelId(manifest.id, schema.schema, field.name, choice));
      }
    }
  }
  for (const step of manifest.steps ?? []) ids.push(stepNameId(manifest.id, step.type));
  for (const widget of manifest.widgets ?? []) {
    ids.push(widgetNameId(manifest.id, widget.type));
    for (const field of widget.fields) {
      ids.push(widgetFieldLabelId(manifest.id, widget.type, field.name));
    }
  }
  return ids;
}

export function allManifestMessageIds(): string[] {
  return MANIFESTS.flatMap(manifestMessageIds);
}
