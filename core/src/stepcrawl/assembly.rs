//! Reconstruct each solid body's Shapr3D folder path from the assembly
//! tree: walk the NEXT_ASSEMBLY_USAGE_OCCURRENCE chain upward from a
//! body's containing product to the root product.

use super::entities::{refs, typed, unquote};
use std::collections::{HashMap, HashSet};

pub struct BrepContainer {
    pub rep_id: i64,
    /// Fallback folder prefix used when `container_pd` can't resolve a
    /// product-definition id for this container (see `extract_parts`).
    pub container_name: String,
    pub solid_ids: Vec<i64>,
}

/// A body's geometry rep (ADVANCED_BREP_SHAPE_REPRESENTATION) is a sibling
/// of the plain SHAPE_REPRESENTATION that's actually linked to the product
/// definition via SHAPE_DEFINITION_REPRESENTATION -- they share a context
/// id. See `container_pd`.
pub struct AssemblyIndex {
    products: HashMap<i64, String>,
    formation_to_product: HashMap<i64, i64>,
    pd_to_formation: HashMap<i64, i64>,
    pds_to_definition: HashMap<i64, i64>,
    pds_by_context: HashMap<i64, i64>,
    rep_context: HashMap<i64, i64>,
    nauo_by_child: HashMap<i64, (i64, String)>,
}

pub fn build_indices(entities: &HashMap<i64, String>) -> (Vec<BrepContainer>, AssemblyIndex) {
    let mut products = HashMap::new();
    let mut formation_to_product = HashMap::new();
    let mut pd_to_formation = HashMap::new();
    let mut pds_to_definition = HashMap::new();
    let mut sdr_by_rep = HashMap::new();
    let mut rep_context = HashMap::new();
    let mut breps_by_container = Vec::new();
    let mut nauo_by_child = HashMap::new();

    for &id in entities.keys() {
        let (typ, args) = typed(entities, id);
        let typ = match typ {
            Some(t) => t,
            None => continue,
        };
        match typ.as_str() {
            "PRODUCT" => {
                products.insert(id, unquote(&args[0]));
            }
            "PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE" | "PRODUCT_DEFINITION_FORMATION" => {
                formation_to_product.insert(id, refs(&args[2])[0]);
            }
            "PRODUCT_DEFINITION" => {
                pd_to_formation.insert(id, refs(&args[2])[0]);
            }
            "PRODUCT_DEFINITION_SHAPE" => {
                pds_to_definition.insert(id, refs(&args[2])[0]);
            }
            "SHAPE_DEFINITION_REPRESENTATION" => {
                let pds_id = refs(&args[0])[0];
                let rep_id = refs(&args[1])[0];
                sdr_by_rep.insert(rep_id, pds_id);
            }
            "SHAPE_REPRESENTATION" | "ADVANCED_BREP_SHAPE_REPRESENTATION" => {
                rep_context.insert(id, refs(&args[2])[0]);
                if typ == "ADVANCED_BREP_SHAPE_REPRESENTATION" {
                    let name = unquote(&args[0]);
                    let solid_ids = refs(&args[1]);
                    breps_by_container.push(BrepContainer { rep_id: id, container_name: name, solid_ids });
                }
            }
            "NEXT_ASSEMBLY_USAGE_OCCURRENCE" => {
                let name = unquote(&args[1]);
                let parent_pd = refs(&args[3])[0];
                let child_pd = refs(&args[4])[0];
                nauo_by_child.insert(child_pd, (parent_pd, name));
            }
            _ => {}
        }
    }

    let mut pds_by_context = HashMap::new();
    for (&rep_id, &pds_id) in sdr_by_rep.iter() {
        if let Some(&ctx) = rep_context.get(&rep_id) {
            pds_by_context.insert(ctx, pds_id);
        }
    }

    let index = AssemblyIndex {
        products,
        formation_to_product,
        pd_to_formation,
        pds_to_definition,
        pds_by_context,
        rep_context,
        nauo_by_child,
    };

    (breps_by_container, index)
}

impl AssemblyIndex {
    fn pd_to_product_name(&self, pd_id: i64) -> Option<&str> {
        let formation = self.pd_to_formation.get(&pd_id)?;
        let product = self.formation_to_product.get(formation)?;
        self.products.get(product).map(|s| s.as_str())
    }

    pub fn container_pd(&self, rep_id: i64) -> Option<i64> {
        let ctx = self.rep_context.get(&rep_id)?;
        let pds_id = self.pds_by_context.get(ctx)?;
        self.pds_to_definition.get(pds_id).copied()
    }

    pub fn ancestor_path(&self, pd_id: i64) -> Vec<String> {
        let mut path = Vec::new();
        let mut cur = pd_id;
        let mut seen = HashSet::new();
        while let Some((parent, name)) = self.nauo_by_child.get(&cur) {
            if !seen.insert(cur) {
                break;
            }
            path.push(name.clone());
            cur = *parent;
        }
        if let Some(root_name) = self.pd_to_product_name(cur) {
            path.push(root_name.to_string());
        }
        path.reverse();
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestor_path_walks_nauo_chain_to_root_product() {
        let mut entities: HashMap<i64, String> = HashMap::new();
        // Root product chain: PRODUCT #2 -> FORMATION #3 -> PD #1 (root pd)
        entities.insert(2, "PRODUCT('Living Room Built-In','','',(#1))".to_string());
        entities.insert(3, "PRODUCT_DEFINITION_FORMATION('','',#2)".to_string());
        entities.insert(1, "PRODUCT_DEFINITION('','',#3,#1)".to_string());

        // NAUO chain: root(#1) -> Bench(#11) -> Carcasses(#12) -> Carcass A(#13)
        entities.insert(20, "NEXT_ASSEMBLY_USAGE_OCCURRENCE('N1','Bench','',#1,#11,$)".to_string());
        entities.insert(21, "NEXT_ASSEMBLY_USAGE_OCCURRENCE('N2','Carcasses','',#11,#12,$)".to_string());
        entities.insert(22, "NEXT_ASSEMBLY_USAGE_OCCURRENCE('N3','Carcass A','',#12,#13,$)".to_string());

        // Container linkage for Carcass A's brep representation
        entities.insert(14, "PRODUCT_DEFINITION_SHAPE('','',#13)".to_string());
        entities.insert(15, "SHAPE_DEFINITION_REPRESENTATION(#14,#16)".to_string());
        entities.insert(16, "SHAPE_REPRESENTATION('',(#17),#18)".to_string());
        entities.insert(19, "ADVANCED_BREP_SHAPE_REPRESENTATION('CarcassA_Brep',(#100,#200),#18)".to_string());

        let (breps, index) = build_indices(&entities);
        assert_eq!(breps.len(), 1);
        let brep = &breps[0];
        assert_eq!(brep.solid_ids, vec![100, 200]);

        let pd_id = index.container_pd(brep.rep_id).expect("container_pd should resolve");
        assert_eq!(pd_id, 13);

        let path = index.ancestor_path(pd_id);
        assert_eq!(path, vec!["Living Room Built-In", "Bench", "Carcasses", "Carcass A"]);
    }
}
