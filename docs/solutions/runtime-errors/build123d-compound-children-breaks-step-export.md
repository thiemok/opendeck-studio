---
title: build123d Compound(children=...) re-parents parts and breaks their STEP export
date: 2026-11-17
category: runtime-errors
module: cad (opendeck_case)
problem_type: runtime_error
component: tooling
symptoms:
  - "export_step raises RuntimeError: Failed to write STEP file"
  - "The underlying error in the traceback is Unknown Compound type"
  - "Only parts that were earlier passed to Compound(children=...) fail; the same part exports fine in isolation"
root_cause: wrong_api
resolution_type: code_fix
severity: medium
retire_when: "build123d stops re-parenting shapes passed as children, or export_step accepts a part that has a parent; check by exporting a Box after putting it in Compound(children=[...])"
tags: [build123d, compound, step-export, assembly, cad]
---

# build123d Compound(children=...) re-parents parts and breaks their STEP export

## Problem

In the case-model build (`cad/`, build123d 0.13.0), a check that only needed the bounding box of all tiles together made the tiles children of a new compound. Every tile then failed STEP export later in the same build, so `case build thi-82` crashed.

## Symptoms

- `export_step(part, path)` raises `RuntimeError: Failed to write STEP file`; the pytest traceback's inner error was `Unknown Compound type`.
- It fails only for parts that were passed to `Compound(children=[...])` earlier. The same parts, built fresh in a scratch script, export fine. That made the failure look order-dependent.

## What Didn't Work

- Suspecting the export code: the export tests passed, and exporting every part of a fresh variant one by one worked. The failure only appeared in the full build, where the outline check runs before export.
- Suspecting the `Pos(-bounding_box().min) * part` translation in `export.py`: it was changed in the same pass, but a Box moved that way exports fine.

## Solution

Use the plain constructor, which wraps the shapes without building an assembly tree:

```python
# Before: the tiles become children of the compound and can no longer export to STEP
size = Compound(children=list(tiles)).bounding_box().size

# After: a throwaway compound; the tiles keep parent None
size = Compound(list(tiles)).bounding_box().size
```

Both uses in the package now do this: the assembled-outline check (`cad/src/opendeck_case/checks.py:27`) and the assembly preview (`cad/src/opendeck_case/preview.py:45`). The preview had the same latent bug, but it ran after export, so it never showed.

## Why This Works

`Compound(children=...)` builds an assembly: it assigns `self.children` (`build123d/topology/composite.py:175` in the installed 0.13.0 package), and the anytree node mixin sets each child's `parent` to the new compound. STEP export walks a part's `parent` to place it in an assembly (`build123d/exporters3d.py:214-222`, installed package), so a lone part in that state fails. Reproduced in this session: after `Compound(children=[a, b])`, `a.parent is c` is true and `export_step(a, ...)` fails. `Compound([d])` leaves `d.parent` as `None`.

The mutation is a side effect of a call that looks read-only. Any code that builds a compound "just to measure" can break export of the inputs later on.

## Prevention

- Use `Compound(shapes)` for a temporary group (a combined bounding box, a preview, an intersection). Use `Compound(children=...)` only when you mean to build an assembly that owns the parts.
- `cad/tests/test_build.py` runs the full `thi-82` build through the CLI, so outline check and export run in production order. Keep that end-to-end test. The unit tests, each on fresh parts, did not catch this.
- When simplifying, don't swap `sum(parts)` or a manual bounding-box union for `Compound(children=parts)` on parts that are used afterwards.

## Related Issues

- Plan: `docs/plans/2026-11-17-0824-feat-case-model-setup-plan.md` (THI-82 case-model setup).
