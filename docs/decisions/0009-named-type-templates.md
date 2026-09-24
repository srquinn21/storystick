# ADR-0009: User-Defined Type Templates via Bracketed Name Tags

**Status:** Accepted

## Context

Some parts share a common role that implies a bundle of defaults, not just one property. A carcass, for example, might imply 3/4" plywood for its panels and 1/4" plywood for its back, several parts, several defaults, from one concept. Assigning material and thickness to each part individually, one at a time, is the same repetitive-annotation problem grain direction and tool assignment already solve for a single property, but here it's a group of properties across a group of parts.

Part and folder names are already the stable identity key parts get matched on across re-imports (see [ADR-0016](0016-stable-part-identity.md)). A naming convention can ride along with that for free, no new sync mechanism needed, just an additional parsing step over a string the tool already treats as meaningful.

## Decision

Support user-defined named type templates, triggered by an explicit bracketed tag in a part or folder name, e.g. `[Carcass]`. A template bundles whatever defaults the user wants under that name; assigning a value once for the template (e.g., "what ply for Carcass") cascades to every part tagged with it.

Brackets are a deliberate opt-in marker of intent, not decoration. `[carcass]` means "apply the Carcass template here." Plain `carcass` appearing in a name, with no brackets, means nothing to the tool, the user is using that word for their own display or organizational purposes and explicitly not opting into templating. Casing is normalized within a tag, `[Carcass]` and `[CARCASS]` match the same template, but bracket presence itself is not normalized or inferred, only literally bracketed tokens are read as tags.

Where a template implies different defaults for different roles within the same type, e.g. panel vs. backer needing different thicknesses, that role needs its own explicit tag too. Not inferred geometrically.

Templates are not hardcoded. What "Carcass" means, and what it implies, is authored by the user, not built into the tool. The specific mechanism for authoring a template, a form, a reference STEP file that describes a typical build, something else entirely, is not decided here. Left open, see Consequences.

The tool may ship with a suggested starter list of common template *names*, drawn from standard, near-universal cabinetmaking nomenclature (Carcass, Face Frame, Toe Kick, and so on, see `docs/features/named-type-templates.md`). This is a naming convenience only, the same treatment already given to Supported Tools & Techniques, "a starter set of examples, not an exhaustive list." A suggested name still implies nothing on its own; its material, thickness, and role defaults stay exactly as user-defined and empty as a template the user typed from scratch. Pre-populating the name saves retyping a term nobody's realistically going to rename; it does not pre-populate meaning.

## Alternatives Considered

Hardcoding built-in type templates (the tool ships knowing what a "Carcass" is). Rejected: that's one user's personal building convention, not a universal rule, someone else's carcass is a different thickness or solid wood instead of ply. Baking it in overfits the tool to one workflow.

Inferring a part's role within a type geometrically instead of via an explicit tag, e.g. guessing "the large thin one is probably the backer." Rejected: getting this wrong assigns the wrong material thickness, a real BOM/ordering error, not a cosmetic annotation gap. Worth an explicit tag, not a guess, given what's at stake.

## Consequences

The tool needs a tag-parsing step over part and folder names, extracting bracketed tokens, case-insensitive within the brackets, and a store for user-authored templates and their defaults. Both compose with the existing name-based stable-key architecture (see [ADR-0016](0016-stable-part-identity.md)) rather than requiring anything new.

Left open, not decided here:
- The actual UX for authoring a template (form-based, derived from a reference STEP file, or something else).
- Whether the tool validates/flags a part whose name suggests a type but isn't actually tagged (e.g., plain `carcass`, no brackets, sitting next to tagged siblings), versus trusting the user's bracket usage as-is.
