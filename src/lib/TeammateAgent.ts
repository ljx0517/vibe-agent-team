/**
 * TeammateAgent - TypeScript class for interacting with teammate agents
 *
 * This class provides a simple interface to:
 * - Start a teammate agent from database configuration
 * - Send messages to the running agent
 * - Listen for output events
 * - Stop the agent when done
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

/**
 * Message from Claude stream
 */
export interface ClaudeStreamMessage {
  type: string;
  content?: string;
  message?: {
    role: string;
    content: string;
  };
  [key: string]: unknown;
}

/**
 * Callback types for agent events
 */
export type OutputCallback = (message: string) => void;
export type ErrorCallback = (error: string) => void;
export type CompleteCallback = (success: boolean) => void;

/**
 * TeammateAgent class
 *
 * Provides a high-level interface to manage a teammate agent process.
 * The agent runs continuously and can receive messages via stdin.
 */
export class TeammateAgent {
  private agentId: string;
  private projectPath: string;
  private runId: string | null = null;
  private model?: string;

  // Event callbacks
  private outputCallbacks: OutputCallback[] = [];
  private errorCallbacks: ErrorCallback[] = [];
  private completeCallbacks: CompleteCallback[] = [];

  // Event listeners (for cleanup)
  private unlistenOutput: UnlistenFn | null = null;
  private unlistenError: UnlistenFn | null = null;
  private unlistenComplete: UnlistenFn | null = null;

  /**
   * Create a new TeammateAgent instance
   *
   * @param agentId - The ID of the agent from the database
   * @param projectPath - Path to the project directory
   * @param model - Optional model override (e.g., 'sonnet', 'haiku')
   */
  constructor(agentId: string, projectPath: string, model?: string) {
    this.agentId = agentId;
    this.projectPath = projectPath;
    this.model = model;
  }

  /**
   * Start the teammate agent
   *
   * @returns Promise<void>
   * @throws Error if starting fails
   */
  async start(): Promise<void> {
    console.log(`[TeammateAgent] Starting agent: ${this.agentId} in ${this.projectPath}`);

    try {
      // Call the Tauri command to start the agent
      this.runId = await invoke<string>("start_teammate_agent", {
        agentId: this.agentId,
        projectPath: this.projectPath,
        model: this.model,
      });

      console.log(`[TeammateAgent] Started with runId: ${this.runId}`);

      // Set up event listeners
      await this.setupEventListeners();
    } catch (error) {
      console.error(`[TeammateAgent] Failed to start:`, error);
      throw error;
    }
  }

  /**
   * Set up event listeners for agent output
   */
  private async setupEventListeners(): Promise<void> {
    if (!this.runId) {
      throw new Error("Agent not started - no runId");
    }

    // Listen for output events
    this.unlistenOutput = await listen<string>(
      `teammate-output:${this.runId}`,
      (event) => {
        console.log(`[TeammateAgent] Output:`, event.payload);
        this.outputCallbacks.forEach((cb) => cb(event.payload));
      }
    );

    // Listen for error events
    this.unlistenError = await listen<string>(
      `teammate-error:${this.runId}`,
      (event) => {
        console.error(`[TeammateAgent] Error:`, event.payload);
        this.errorCallbacks.forEach((cb) => cb(event.payload));
      }
    );

    // Listen for completion events
    this.unlistenComplete = await listen<boolean>(
      `teammate-complete:${this.runId}`,
      (event) => {
        console.log(`[TeammateAgent] Complete:`, event.payload);
        this.completeCallbacks.forEach((cb) => cb(event.payload));
        // Clean up listeners after completion
        this.cleanup();
      }
    );
  }

  /**
   * Send a message to the running agent
   *
   * @param content - The message content to send
   * @returns Promise<void>
   * @throws Error if agent is not running
   */
  async sendMessage(content: string): Promise<void> {
    if (!this.runId) {
      throw new Error("Agent not started - call start() first");
    }

    console.log(`[TeammateAgent] Sending message:`, content);

    await invoke("send_to_teammate", {
      runId: this.runId,
      message: content,
    });
  }

  /**
   * Register a callback for output messages
   *
   * @param callback - Function to call when output is received
   */
  onOutput(callback: OutputCallback): void {
    this.outputCallbacks.push(callback);
  }

  /**
   * Register a callback for error messages
   *
   * @param callback - Function to call when an error occurs
   */
  onError(callback: ErrorCallback): void {
    this.errorCallbacks.push(callback);
  }

  /**
   * Register a callback for completion
   *
   * @param callback - Function to call when the agent completes
   */
  onComplete(callback: CompleteCallback): void {
    this.completeCallbacks.push(callback);
  }

  /**
   * Remove a callback from output listeners
   *
   * @param callback - The callback to remove
   */
  removeOutputListener(callback: OutputCallback): void {
    const index = this.outputCallbacks.indexOf(callback);
    if (index > -1) {
      this.outputCallbacks.splice(index, 1);
    }
  }

  /**
   * Remove a callback from error listeners
   *
   * @param callback - The callback to remove
   */
  removeErrorListener(callback: ErrorCallback): void {
    const index = this.errorCallbacks.indexOf(callback);
    if (index > -1) {
      this.errorCallbacks.splice(index, 1);
    }
  }

  /**
   * Remove a callback from completion listeners
   *
   * @param callback - The callback to remove
   */
  removeCompleteListener(callback: CompleteCallback): void {
    const index = this.completeCallbacks.indexOf(callback);
    if (index > -1) {
      this.completeCallbacks.splice(index, 1);
    }
  }

  /**
   * Stop the running agent
   *
   * @returns Promise<boolean> - True if successfully stopped
   * @throws Error if agent is not running
   */
  async kill(): Promise<boolean> {
    if (!this.runId) {
      throw new Error("Agent not started - call start() first");
    }

    console.log(`[TeammateAgent] Killing agent: ${this.runId}`);

    try {
      const result = await invoke<boolean>("stop_teammate_agent", {
        runId: this.runId,
      });

      this.cleanup();
      return result;
    } catch (error) {
      console.error(`[TeammateAgent] Failed to kill:`, error);
      throw error;
    }
  }

  /**
   * Get the current status of the agent
   *
   * @returns Promise<string | null> - 'running' or null if not running
   */
  async getStatus(): Promise<string | null> {
    if (!this.runId) {
      return null;
    }

    try {
      const status = await invoke<string | null>("get_teammate_status", {
        runId: this.runId,
      });
      return status;
    } catch (error) {
      console.error(`[TeammateAgent] Failed to get status:`, error);
      return null;
    }
  }

  /**
   * Get the run ID of the current session
   *
   * @returns string | null
   */
  getRunId(): string | null {
    return this.runId;
  }

  /**
   * Check if the agent is currently running
   *
   * @returns boolean
   */
  isRunning(): boolean {
    return this.runId !== null;
  }

  /**
   * Clean up event listeners
   */
  private cleanup(): void {
    if (this.unlistenOutput) {
      this.unlistenOutput();
      this.unlistenOutput = null;
    }
    if (this.unlistenError) {
      this.unlistenError();
      this.unlistenError = null;
    }
    if (this.unlistenComplete) {
      this.unlistenComplete();
      this.unlistenComplete = null;
    }

    this.runId = null;
  }

  /**
   * Destroy the agent and clean up all resources
   * Should be called when done using the agent
   */
  destroy(): void {
    if (this.runId) {
      this.kill().catch(console.error);
    }
    this.cleanup();
    this.outputCallbacks = [];
    this.errorCallbacks = [];
    this.completeCallbacks = [];
  }
}

export default TeammateAgent;
