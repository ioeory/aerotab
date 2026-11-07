import type { RpcClient } from './rpc';

/** Local-only ordinary script definitions. Never copy SSH/auth objects here. */
export interface ScriptInput { id: string; name: string; command: string; description: string; profile_id: string | null }
export interface SavedScript extends ScriptInput { version: number; updated_at: number }
export interface ScriptConfirmation { token: string; profile_id: string; target: string; script_name: string; command: string; timeout_seconds: number; output_limit: number }
export type ScriptStatus = 'running' | 'completed' | 'cancelled' | 'timed_out' | 'output_limit' | 'failed';
export interface ScriptTask { id: string; script_id: string; profile_id: string; target: string; status: ScriptStatus; stdout: string; stderr: string; exit_code: number | null; error: string | null; truncated: boolean }
export function scriptPayload(draft: ScriptInput): ScriptInput {
  return { id: draft.id, name: draft.name.trim(), command: draft.command, description: draft.description, profile_id: draft.profile_id || null };
}

/** prepare never executes; only the explicit second action consumes a one-use token. */
export class ScriptRunner {
  confirmation: ScriptConfirmation | null = null;
  task: ScriptTask | null = null;
  private generation = 0;
  constructor(private rpc: RpcClient) {}
  async prepare(id: string, profile_id: string, timeout_seconds = 30) {
    const generation = ++this.generation;
    this.confirmation = null;
    const result = await this.rpc.call<ScriptConfirmation>('scripts.prepare', { id, profile_id, timeout_seconds });
    if (generation === this.generation) this.confirmation = result;
    return this.confirmation;
  }
  dismiss() { this.generation++; this.confirmation = null; }
  async confirm() {
    const confirmation = this.confirmation;
    if (!confirmation) throw new Error('Explicit target confirmation required');
    this.dismiss();
    this.task = await this.rpc.call<ScriptTask>('scripts.run', { token: confirmation.token });
    return this.task;
  }
  async refresh() {
    const previous = this.task;
    if (previous) {
      const result = await this.rpc.call<ScriptTask>('scripts.status', { id: previous.id });
      if (this.task === previous) this.task = result;
    }
    return this.task;
  }
  async cancel() {
    const previous = this.task;
    if (previous) {
      const result = await this.rpc.call<ScriptTask>('scripts.cancel', { id: previous.id });
      if (this.task?.id === previous.id) this.task = result;
    }
    return this.task;
  }
}
