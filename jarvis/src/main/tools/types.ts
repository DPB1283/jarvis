import type Anthropic from "@anthropic-ai/sdk";
import type { z } from "zod";
import type { Risk } from "../../shared/events";

export type ToolOutput = string | Anthropic.ToolResultBlockParam["content"];

export interface JarvisTool<S extends z.ZodType = z.ZodType> {
  name: string;
  description: string;
  schema: S;
  /** Static risk, or computed from the input (e.g. overwrite vs. create). */
  risk: Risk | ((input: z.infer<S>) => Risk);
  /** One-line human description shown in the approval chip / spoken aloud. */
  summary: (input: z.infer<S>) => string;
  run: (input: z.infer<S>) => Promise<ToolOutput>;
}

export function defineTool<S extends z.ZodType>(t: JarvisTool<S>): JarvisTool<S> {
  return t;
}
