import { randomUUID } from "node:crypto";
import type { Risk } from "../../shared/events";
import { bus } from "../bus";
import { getSettings } from "../config";

export type Approver = (req: { id: string; tool: string; summary: string; risk: Risk }) => Promise<boolean>;

const APPROVAL_TIMEOUT_MS = 60_000;

let pendingCount = 0;
/** True while Jarvis is waiting on a yes/no. */
export const approvalPending = () => pendingCount > 0;

/** Default approver: asks the HUD / voice over the bus and waits for approval_response. */
const busApprover: Approver = (req) =>
  new Promise((resolve) => {
    pendingCount++;
    let settled = false;
    const done = (ok: boolean) => {
      if (settled) return;
      settled = true;
      pendingCount--;
      clearTimeout(timer);
      offResp();
      offInt();
      bus.emit("approval_response", { id: req.id, ok }); // lets every UI clear the chip
      resolve(ok);
    };
    const offResp = bus.on("approval_response", (r) => {
      if (r.id === req.id) done(r.ok);
    });
    // "Jarvis, yes" fires the wake word (barge_in); that must not cancel the question being answered.
    const offInt = bus.on("interrupt", ({ reason }) => {
      if (reason !== "barge_in") done(false);
    });
    const timer = setTimeout(() => done(false), APPROVAL_TIMEOUT_MS);
    bus.emit("approval_request", req);
  });

let approver: Approver = busApprover;

/** The CLI swaps in a terminal prompt. */
export function setApprover(fn: Approver): void {
  approver = fn;
}

export function needsApproval(risk: Risk): boolean {
  switch (getSettings().safetyMode) {
    case "ask_all":
      return true;
    case "ask_unsafe":
      return risk === "unsafe";
    case "full_access":
      return false;
  }
}

/** Returns true if the action may run. */
export async function authorize(tool: string, summary: string, risk: Risk): Promise<boolean> {
  if (!needsApproval(risk)) return true;
  return approver({ id: randomUUID(), tool, summary, risk });
}
