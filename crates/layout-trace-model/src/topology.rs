use serde::{Deserialize, Deserializer, Serialize};

/// A stable description of how a route passes the labeled obstacles on a layer.
///
/// This is deliberately independent of any triangulation. The triangulation is
/// allowed to flip or be rebuilt while this word remains unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct HomotopyWord(Vec<CutCrossing>);

impl<'de> Deserialize<'de> for HomotopyWord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let crossings = Vec::<CutCrossing>::deserialize(deserializer)?;
        Ok(Self::reduced(crossings))
    }
}

impl HomotopyWord {
    pub fn reduced(crossings: impl IntoIterator<Item = CutCrossing>) -> Self {
        let mut result: Vec<CutCrossing> = Vec::new();
        for crossing in crossings {
            if result.last().is_some_and(|last| last.cancels(&crossing)) {
                result.pop();
            } else {
                result.push(crossing);
            }
        }
        Self(result)
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn crossings(&self) -> &[CutCrossing] {
        &self.0
    }

    pub fn is_reduced(&self) -> bool {
        self.0.windows(2).all(|pair| !pair[0].cancels(&pair[1]))
    }

    pub fn reversed(&self) -> Self {
        Self(self.0.iter().rev().map(CutCrossing::inverse).collect())
    }

    pub fn extended(&self, crossings: impl IntoIterator<Item = CutCrossing>) -> Self {
        Self::reduced(self.0.iter().cloned().chain(crossings))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutCrossing {
    /// Stable obstacle-component or cut identifier, never a mesh-cell ID.
    pub obstacle: String,
    pub direction: CrossingDirection,
}

impl CutCrossing {
    pub fn inverse(&self) -> Self {
        Self {
            obstacle: self.obstacle.clone(),
            direction: self.direction.inverse(),
        }
    }

    fn cancels(&self, other: &Self) -> bool {
        self.obstacle == other.obstacle && self.direction != other.direction
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossingDirection {
    Positive,
    Negative,
}

impl CrossingDirection {
    fn inverse(self) -> Self {
        match self {
            Self::Positive => Self::Negative,
            Self::Negative => Self::Positive,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalSector {
    pub component: String,
    /// Initially a pin ID. Later this may name an edge interval or a flexible
    /// terminal region without changing the route-class representation.
    pub sector: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteClass {
    pub layer: String,
    pub from: TerminalSector,
    pub to: TerminalSector,
    pub homotopy: HomotopyWord,
}

impl RouteClass {
    pub fn direct(layer: impl Into<String>, from: TerminalSector, to: TerminalSector) -> Self {
        Self {
            layer: layer.into(),
            from,
            to,
            homotopy: HomotopyWord::empty(),
        }
    }

    pub fn reversed(&self) -> Self {
        Self {
            layer: self.layer.clone(),
            from: self.to.clone(),
            to: self.from.clone(),
            homotopy: self.homotopy.reversed(),
        }
    }
}

/// Stable-ish identity for a gate in a disposable corridor mesh.
///
/// Boundary-feature IDs are sorted so a rebuilt mesh produces the same key.
/// `direction` records traversal relative to that canonical ordering.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GateKey {
    pub layer: String,
    pub boundary_feature_a: String,
    pub boundary_feature_b: String,
    pub direction: GateDirection,
}

impl GateKey {
    pub fn new(
        layer: impl Into<String>,
        feature_a: impl Into<String>,
        feature_b: impl Into<String>,
        direction: GateDirection,
    ) -> Self {
        let feature_a = feature_a.into();
        let feature_b = feature_b.into();
        if feature_a <= feature_b {
            Self {
                layer: layer.into(),
                boundary_feature_a: feature_a,
                boundary_feature_b: feature_b,
                direction,
            }
        } else {
            Self {
                layer: layer.into(),
                boundary_feature_a: feature_b,
                boundary_feature_b: feature_a,
                direction: direction.inverse(),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateDirection {
    Forward,
    Reverse,
}

impl GateDirection {
    fn inverse(self) -> Self {
        match self {
            Self::Forward => Self::Reverse,
            Self::Reverse => Self::Forward,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortalTraversal {
    pub gate: GateKey,
    pub lane: usize,
}

/// A route realization that is valid only for the stated placement and mesh.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorridorEmbedding {
    pub placement_revision: u64,
    pub corridor_revision: u64,
    pub portals: Vec<PortalTraversal>,
}

impl CorridorEmbedding {
    pub fn is_fresh_for(&self, placement_revision: u64, corridor_revision: u64) -> bool {
        self.placement_revision == placement_revision && self.corridor_revision == corridor_revision
    }
}

/// Capacity accounting for an ordered, single-layer trace bundle.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrderedBundle {
    pub lanes: Vec<Lane>,
}

impl OrderedBundle {
    /// Includes one clearance at each corridor wall and between adjacent lanes.
    pub fn required_width(&self, clearance: f64) -> f64 {
        if self.lanes.is_empty() {
            0.0
        } else {
            self.lanes.iter().map(|lane| lane.width).sum::<f64>()
                + clearance * (self.lanes.len() + 1) as f64
        }
    }

    pub fn fits(&self, usable_width: f64, clearance: f64) -> bool {
        self.required_width(clearance) <= usable_width
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub net: String,
    pub width: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crossing(obstacle: &str, direction: CrossingDirection) -> CutCrossing {
        CutCrossing {
            obstacle: obstacle.into(),
            direction,
        }
    }

    #[test]
    fn reduction_is_order_sensitive_and_cancels_inverses() {
        use CrossingDirection::{Negative, Positive};
        let word = HomotopyWord::reduced([
            crossing("a", Positive),
            crossing("b", Positive),
            crossing("b", Negative),
            crossing("a", Negative),
            crossing("c", Positive),
        ]);
        assert_eq!(word.crossings(), &[crossing("c", Positive)]);

        let non_commuting = HomotopyWord::reduced([
            crossing("a", Positive),
            crossing("b", Positive),
            crossing("a", Negative),
        ]);
        assert_eq!(non_commuting.crossings().len(), 3);
    }

    #[test]
    fn reversing_a_route_inverts_crossing_order() {
        use CrossingDirection::{Negative, Positive};
        let class = RouteClass {
            layer: "top".into(),
            from: TerminalSector {
                component: "u1".into(),
                sector: "p1".into(),
            },
            to: TerminalSector {
                component: "u2".into(),
                sector: "p2".into(),
            },
            homotopy: HomotopyWord::reduced([crossing("a", Positive), crossing("b", Negative)]),
        };
        let reversed = class.reversed();
        assert_eq!(reversed.from, class.to);
        assert_eq!(
            reversed.homotopy.crossings(),
            &[crossing("b", Positive), crossing("a", Negative)]
        );
    }

    #[test]
    fn gate_keys_survive_boundary_feature_order() {
        assert_eq!(
            GateKey::new("top", "a", "b", GateDirection::Forward),
            GateKey::new("top", "b", "a", GateDirection::Reverse)
        );
    }

    #[test]
    fn ordered_bundle_accounts_for_walls_and_lane_clearance() {
        let bundle = OrderedBundle {
            lanes: vec![
                Lane {
                    net: "n1".into(),
                    width: 0.2,
                },
                Lane {
                    net: "n2".into(),
                    width: 0.3,
                },
            ],
        };
        assert!((bundle.required_width(0.1) - 0.8).abs() < 1.0e-12);
        assert!(bundle.fits(0.8, 0.1));
        assert!(!bundle.fits(0.79, 0.1));
    }

    #[test]
    fn deserialization_canonicalizes_a_homotopy_word() {
        let word: HomotopyWord = serde_json::from_str(
            r#"[
                {"obstacle":"a","direction":"positive"},
                {"obstacle":"a","direction":"negative"}
            ]"#,
        )
        .unwrap();
        assert!(word.crossings().is_empty());
        assert!(word.is_reduced());
    }
}
