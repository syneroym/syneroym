import type { ExtensionRef, FieldType } from "../extensions/registry";

export interface DefinedChoice {
  code: string;
  text: string;
}

export interface DefinedField {
  name: string;
  type: FieldType;
  required?: boolean;
  max_len?: number;
  min?: number;
  max?: number;
  label?: string;
  choices?: DefinedChoice[];
}

export interface WidgetDefinition {
  extension: ExtensionRef;
  type: string;
  semantic: string;
  prompt: string;
  fields: DefinedField[];
}

export type Direction = "incoming" | "outgoing";
export type StoredState = "pending" | "sent" | "delivered" | "deposited" | "failed";

export interface ChoiceCount {
  field: string;
  choice: string;
  count: number;
}

export interface AnswerView {
  message_id: string;
  author: string;
  direction: Direction;
  sender_timestamp_ms: number;
  languages?: string[];
  data: Record<string, unknown>;
  state: StoredState;
  last_error?: string | null;
}

export interface WidgetView {
  widget_id: string;
  author: string;
  direction: Direction;
  sender_timestamp_ms: number;
  languages?: string[];
  definition?: WidgetDefinition | null;
  refused?: string | null;
  checked: boolean;
  counts: ChoiceCount[];
  answers: AnswerView[];
  ignored_answers: number;
  state: StoredState;
  last_error?: string | null;
}

export interface WidgetListOutput {
  widgets: WidgetView[];
  complete: boolean;
}

export const WIDGET_CONTENT_TYPE = "application/vnd.roym.widget+json";
export const WIDGET_ANSWER_CONTENT_TYPE = "application/vnd.roym.widget-answer+json";

export interface WidgetRenderOptions {
  conversation: string;
  refresh?: () => Promise<void> | void;
  nameOf?: (addr: string) => string;
  languages?: () => string[];
  isRestored?: (messageId: string) => boolean;
}

export function widgetPrompt(body: string | undefined | null): string | null {
  if (!body) return null;
  try {
    const parsed = JSON.parse(body);
    const prompt = parsed?.widget?.prompt;
    if (typeof prompt === "string" && prompt.trim()) {
      return prompt;
    }
  } catch {
    // Malformed JSON
  }
  return null;
}

export function pollDefinition(question: string, options: string[]): WidgetDefinition {
  const choices: DefinedChoice[] = options.map((opt, i) => ({
    code: `o${i + 1}`,
    text: opt,
  }));
  return {
    extension: { id: "poll", version: 1 },
    type: "poll/single",
    semantic: "poll",
    prompt: question,
    fields: [
      {
        name: "option",
        type: "choice",
        required: true,
        choices,
      },
    ],
  };
}
