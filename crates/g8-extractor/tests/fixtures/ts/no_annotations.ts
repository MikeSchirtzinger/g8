// NO-detect fixture: TypeScript file with no G8 annotations.

// A plain comment.
// @some_other_decorator() — not G8

export function regularFunction(x: number): number {
  return x + 1;
}

/** JSDoc comment */
export class SomeClass {}
