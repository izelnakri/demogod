/** Where a tape failed. */
export interface Location {
  /** The tape file. */
  file: string;
  /** The line, from 1. */
  line: number;
}

/** A failed recording, with where in the tape it failed when it was the tape's doing. */
export declare class DemogodError extends Error {
  /** The tape line it failed on. */
  location?: Location;
  /** How to fix it, when that is clear. */
  help?: string;
  /** The whole report, the way the command line prints it. */
  text: string;
}

/** A file a recording was saved to. */
export interface Saved {
  /** Where it was written. */
  path: string;
  /** How big it is. */
  bytes: number;
  /** How many distinct frames it has; 1 for a PNG, 0 for an asciicast. */
  frames: number;
  /** How long it plays for, in seconds. */
  duration: number;
}

/** What a recording reports as it goes. */
export type DemogodEvent =
  /** `number` of `total` is what the caption strip shows, `2/5`; a scene without a caption has none. */
  | { type: 'scene'; number: number | null; total: number; title: string | null; detail: string | null }
  | { type: 'action'; name: string }
  | { type: 'warning'; message: string }
  | { type: 'recorded'; duration: number }
  | ({ type: 'saved' } & Saved);

export interface RecordOptions {
  /**
   * Save here instead of the tape's `Output`s: `.gif`, `.mp4`, `.webm`, `.png` (the last frame),
   * `.cast` (asciinema), `.txt` (the terminal's screens as text), or a `directory/` of PNG frames.
   */
  output?: string | string[];
  /**
   * What `Do <name>` in the tape runs, off camera, between two keystrokes. Recording waits for
   * each to finish; one that throws fails the recording on that line.
   */
  actions?: Record<string, () => unknown | Promise<unknown>>;
  /** Called with each scene as it starts, each action, and each file saved. */
  onEvent?: (event: DemogodEvent) => void;
  /** The directory the tape's relative paths are resolved from, for a `tape` source. */
  cwd?: string;
  /** Stops the recording. */
  signal?: AbortSignal;
}

/**
 * A tape written in JavaScript, as a template literal. Indentation common to every line is
 * removed, and nothing is escaped, so it reads exactly as a `.tape` file would.
 */
export declare function tape(strings: TemplateStringsArray, ...values: unknown[]): { source: string };

/** Records a tape and saves it to its `Output` files, or to `options.output` instead. */
export declare function record(tape: string | { source: string }, options?: RecordOptions): Promise<Saved[]>;

/** Reads a tape and checks it can be recorded, without recording anything. */
export declare function check(
  tape: string | { source: string },
  options?: Pick<RecordOptions, 'actions' | 'cwd' | 'signal'>,
): Promise<{ scenes: number; steps: number }>;

/** The names of the built-in themes, for `Set Theme`. */
export declare function themes(): Promise<string[]>;

/** The path of the demogod binary for this machine, or `DEMOGOD_BINARY` when it is set. */
export declare function binaryPath(): string;
