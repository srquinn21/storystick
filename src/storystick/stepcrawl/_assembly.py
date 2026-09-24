"""Reconstruct each solid body's Shapr3D folder path from the assembly
tree: walk the NEXT_ASSEMBLY_USAGE_OCCURRENCE chain upward from a body's
containing product to the root product.
"""

from __future__ import annotations

from storystick.stepcrawl._entities import refs, typed, unquote


def build_indices(entities):
    products = {}  # product_id -> name
    formation_to_product = {}  # formation_id -> product_id
    pd_to_formation = {}  # product_definition_id -> formation_id
    pds_to_definition = {}  # product_definition_shape_id -> definition_id
    sdr_by_rep = {}  # shape_representation_id -> pds_id
    rep_context = {}  # shape_representation_id -> context_id
    breps_by_container = []  # (container_name, [solid_ids])
    nauo_by_child = {}  # child_pd_id -> (parent_pd_id, name)

    for id_, raw in entities.items():
        typ, args = typed(entities, id_)
        if typ == "PRODUCT":
            products[id_] = unquote(args[0])
        elif typ == "PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE" or typ == "PRODUCT_DEFINITION_FORMATION":
            formation_to_product[id_] = refs(args[2])[0]
        elif typ == "PRODUCT_DEFINITION":
            pd_to_formation[id_] = refs(args[2])[0]
        elif typ == "PRODUCT_DEFINITION_SHAPE":
            pds_to_definition[id_] = refs(args[2])[0]
        elif typ == "SHAPE_DEFINITION_REPRESENTATION":
            pds_id = refs(args[0])[0]
            rep_id = refs(args[1])[0]
            sdr_by_rep[rep_id] = pds_id
        elif typ in ("SHAPE_REPRESENTATION", "ADVANCED_BREP_SHAPE_REPRESENTATION"):
            rep_context[id_] = refs(args[2])[0]
            if typ == "ADVANCED_BREP_SHAPE_REPRESENTATION":
                name = unquote(args[0])
                solid_ids = refs(args[1])
                breps_by_container.append((id_, name, solid_ids))
        elif typ == "NEXT_ASSEMBLY_USAGE_OCCURRENCE":
            name = unquote(args[1])
            parent_pd = refs(args[3])[0]
            child_pd = refs(args[4])[0]
            nauo_by_child[child_pd] = (parent_pd, name)

    # A body's geometry rep (ADVANCED_BREP_SHAPE_REPRESENTATION) is a sibling
    # of the plain SHAPE_REPRESENTATION that's actually linked to the product
    # definition via SHAPE_DEFINITION_REPRESENTATION -- they share a context id.
    pds_by_context = {}
    for rep_id, pds_id in sdr_by_rep.items():
        ctx = rep_context.get(rep_id)
        if ctx is not None:
            pds_by_context[ctx] = pds_id

    def pd_to_product_name(pd_id):
        formation = pd_to_formation.get(pd_id)
        product = formation_to_product.get(formation)
        return products.get(product)

    def container_pd(rep_id):
        ctx = rep_context.get(rep_id)
        pds_id = pds_by_context.get(ctx)
        if pds_id is None:
            return None
        return pds_to_definition.get(pds_id)

    def ancestor_path(pd_id):
        path = []
        cur = pd_id
        seen = set()
        while cur in nauo_by_child and cur not in seen:
            seen.add(cur)
            parent, name = nauo_by_child[cur]
            path.append(name)
            cur = parent
        root_name = pd_to_product_name(cur)
        if root_name:
            path.append(root_name)
        return list(reversed(path))

    return breps_by_container, container_pd, ancestor_path
