# Named Type Templates

See [ADR-0009](../decisions/0009-named-type-templates.md) for the mechanism, bracketed tags in part/folder names, user-defined, not hardcoded.

## Starter list

Suggested template names, pre-populated as a naming convenience, standard cabinetmaking nomenclature a user isn't likely to rename. Suggesting the *name* implies nothing about material, thickness, or role, those stay blank until the user defines them, same as a template typed from scratch.

- Carcass
- Face Frame
- Door
- Shelf
- Cap Panel
- Backer
- Divider
- Valence
- Molding
- Drawer Box
- Drawer Front
- Toe Kick
- Stretcher
- Nailer

This list is a starting point, not a ceiling, add, remove, or rename freely. Where a template implies different defaults by role within the same type (e.g. Carcass panels vs. Carcass backer), the role needs its own explicit tag too, not a guess, see ADR-0009's reasoning on why that's not inferred geometrically.
