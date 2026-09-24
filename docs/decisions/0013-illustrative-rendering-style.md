# ADR-0013: Illustrative, Non-Photorealistic Rendering Style

**Status:** Accepted

## Context

Story Stick renders geometry it doesn't author (ADR-0008, ADR-0011), and design itself happens elsewhere, paper, the Concepts app, or Shapr3D. Story Stick's actual purpose is build efficiency and predictability, not client presentation, this is a personal tool with no client-facing use case in scope. That framing settled the question of how much rendering fidelity to chase: what helps judge a design, not what impresses an outside viewer who can't otherwise picture it. Separately, because geometry is imported from CAD rather than approximated, real edge data is available, actual `EDGE_CURVE` entities from STEP, not just a silhouette guessed from a raw mesh.

## Decision

Non-photorealistic, illustrative rendering, closer to SketchUp's default look than Shapr3D's semi-realistic rendering. Two parts: flat or toon shading on faces, no complex lighting, reflections, or environment maps, and real B-rep edges rendered as visible line strokes on top, using actual edge data from imported geometry rather than an approximated silhouette.

Materials: flat RGB color per wood species or paint color as the base. Wood-finished parts get a stylized, illustrative grain-line pattern overlaid, oriented along the part's stored grain-direction metadata. Painted parts carry no grain overlay, flat color only, that absence is itself the visual cue distinguishing painted from wood-grain finish. Grain patterns are stylized per species, not photographic, and not one shared pattern across all species: tight and straight for maple, coarse with visible rays for oak, swirly for walnut, wide rings with occasional knots for pine, and so on, since species differ in grain character, not just color.

## Alternatives Considered

Photorealistic or semi-realistic rendering, matching Shapr3D's own capability, PBR materials, real lighting, reflections. Rejected: that fidelity serves communicating a design to someone who can't otherwise visualize it, typically a client, and Story Stick has no client-facing use case in scope. Where that's ever needed, Shapr3D or the user's own concept work already covers it.

Photographic wood-grain textures per species. Rejected: real asset-sourcing burden, photographing or licensing texture images, plus a tiling-variety problem, since real wood doesn't repeat perfectly and needs multiple photo variants per species to avoid obvious repetition. Also visually inconsistent with flat/toon shading, a photographic texture under cel-style lighting reads as two different rendering registers fighting each other.

One shared, generic stylized grain pattern across all wood species, distinguishing species by color alone. Rejected on two grounds: grain character is real, distinguishing information, not just color, similarly-toned species like ash and oak look nothing alike in grain pattern, and a shared pattern would flatten that away. Also rejected on the assumption that per-species patterns would be costly to justify a shared one, they're stylized and cheap regardless of count, unlike photographic textures.

## Consequences

The render layer needs an edge-extraction step from B-rep data, already implied by parsing STEP edge entities in the first place, a toon or flat shading material, and a small library of stylized grain-line patterns, one per common species, reusable and cheap to author or generate procedurally, oriented per part using the grain-direction metadata already tracked for bin-packing and dado/groove classification, a fourth use of the same data. No photographic texture pipeline is needed. This keeps the render layer cheap to build and fast to run even on a large, 150-plus-part assembly, consistent with everything else scoped narrow on purpose in this project.
