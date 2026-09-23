// app.d.ts — TypeScript surface contract for the playground interpreter core.
// app.js is the runtime; these declarations give consumers full type safety.

export interface WobbleNote {
  /** Total Grammar rung: 2 synonym, 3 wobble, 4 fallback */
  rung: 2 | 3 | 4;
  line: number;
  message: string;
}

export interface RunResult {
  /** lines produced by promote() */
  output: string[];
  /** repairs and fallbacks absorbed by the Total Grammar */
  notes: WobbleNote[];
  /** 100-point grading: wobble −2, fallback −3, floor 50 */
  score: number;
  /** letter grade A–F */
  letter: string;
}

/** Parse and run an Operon subset program (Total Grammar — never throws). */
export function runProgram(src: string): RunResult;
