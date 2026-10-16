//! Truck rules at a real tunnel or bridge on a leg.
//!
//! A crossing is eager leg data, not lazy corridor detail: dispatch and the
//! route graph read it to plan a legal road before the truck leaves, so it
//! has to be known without parsing a corridor. The words travel with the
//! rule and its cited source, the way `TunnelData` carries its own.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::DataError;

/// Which placarded loads a crossing turns away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HazmatRule {
    /// No placarded-load rule is modelled at this crossing.
    None,
    /// Some placarded hazard classes are banned outright. A packaged
    /// placarded load whose classes the game does not name is refused.
    Classes,
    /// Every placarded load is banned.
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadCrossing {
    /// `tunnel` | `bridge`
    pub kind: String,
    /// Spoken name.
    pub name: String,
    pub hazmat: HazmatRule,
    /// Placarded flammable liquid in bulk (a fuel tanker) is banned.
    pub bulk_flammable_banned: bool,
    /// Every cargo tank vehicle is banned, whatever it carries.
    pub tank_banned: bool,
    /// Regulated material carried without placards, in pounds; None means
    /// no such limit is modelled.
    pub regulated_lb_limit: Option<u32>,
    pub max_length_ft: Option<f64>,
    pub max_width_ft: Option<f64>,
    pub max_height_ft: Option<f64>,
    /// The cargo rule as spoken when it turns a load away.
    pub hazmat_text: String,
    /// The size rule as spoken; empty when no size limit is modelled.
    pub size_text: String,
    pub source_url: String,
    pub access_date: String,
    /// Which values were read from the source, derived, or assumed.
    pub basis: String,
}

/// The truck and load a crossing judges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrossingLoad {
    pub length_ft: f64,
    pub width_ft: f64,
    pub height_ft: f64,
    pub placarded: bool,
    pub bulk_flammable: bool,
    pub tank: bool,
    pub regulated_lb: u32,
}

impl CrossingLoad {
    /// A rig with nothing regulated on it.
    pub fn plain(length_ft: f64, width_ft: f64, height_ft: f64) -> Self {
        CrossingLoad {
            length_ft,
            width_ft,
            height_ft,
            placarded: false,
            bulk_flammable: false,
            tank: false,
            regulated_lb: 0,
        }
    }

    /// A stock rig: the full combination with a trailer, the tractor alone
    /// without one, at the default width and height.
    pub fn standard_rig(trailer: bool) -> Self {
        use crate::sim::cross_traffic::{COMBINATION_LENGTH_FT, TRACTOR_LENGTH_FT};
        let length_ft = if trailer {
            COMBINATION_LENGTH_FT
        } else {
            TRACTOR_LENGTH_FT
        };
        CrossingLoad::plain(length_ft, 8.5, 13.5)
    }

    /// The catalog's regulated freight: `hazardous` is placarded packaged
    /// material, `fuel_bulk` is placarded flammable liquid in a cargo tank,
    /// and `chemicals` ride without placards but still count by weight.
    pub fn with_cargo(mut self, cargo_key: &str, tank: bool, cargo_lb: u32) -> Self {
        self.placarded = matches!(cargo_key, "hazardous" | "fuel_bulk");
        self.bulk_flammable = cargo_key == "fuel_bulk";
        self.tank = tank;
        self.regulated_lb = if matches!(cargo_key, "hazardous" | "fuel_bulk" | "chemicals") {
            cargo_lb
        } else {
            0
        };
        self
    }

    /// Whether any crossing rule could apply beyond size.
    pub fn regulated(&self) -> bool {
        self.placarded || self.tank || self.regulated_lb > 0
    }
}

impl RoadCrossing {
    /// The spoken reason this crossing turns the load away, or None.
    /// Size is judged before cargo, as at a real portal.
    pub fn refusal(&self, load: &CrossingLoad) -> Option<&str> {
        let over = |limit: Option<f64>, value: f64| limit.is_some_and(|l| value > l);
        if over(self.max_length_ft, load.length_ft)
            || over(self.max_width_ft, load.width_ft)
            || over(self.max_height_ft, load.height_ft)
        {
            return Some(&self.size_text);
        }
        let hazmat = load.placarded
            && match self.hazmat {
                HazmatRule::All => true,
                HazmatRule::Classes => !load.bulk_flammable || self.bulk_flammable_banned,
                HazmatRule::None => load.bulk_flammable && self.bulk_flammable_banned,
            };
        let regulated = self
            .regulated_lb_limit
            .is_some_and(|limit| load.regulated_lb > limit);
        if hazmat || regulated || (load.tank && self.tank_banned) {
            return Some(&self.hazmat_text);
        }
        None
    }

    pub fn validate(&self, context: &str) -> Result<(), DataError> {
        let bad = |what: &str| DataError::value(format!("{context} crossing {what}"));
        if !matches!(self.kind.as_str(), "tunnel" | "bridge") {
            return Err(bad("has an unknown kind"));
        }
        if self.name.trim().is_empty() || self.basis.trim().is_empty() {
            return Err(bad("has no name or basis"));
        }
        if self.source_url.trim().is_empty() || !self.source_url.starts_with("https://") {
            return Err(bad("has no source URL"));
        }
        NaiveDate::parse_from_str(&self.access_date, "%Y-%m-%d")
            .map_err(|_| bad("has an invalid access date"))?;
        let sized = [self.max_length_ft, self.max_width_ft, self.max_height_ft];
        if sized.iter().flatten().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err(bad("has an invalid size limit"));
        }
        if sized.iter().any(Option::is_some) && self.size_text.trim().is_empty() {
            return Err(bad("has a size limit with no spoken rule"));
        }
        let cargo_rule = self.hazmat != HazmatRule::None
            || self.bulk_flammable_banned
            || self.tank_banned
            || self.regulated_lb_limit.is_some();
        if cargo_rule && self.hazmat_text.trim().is_empty() {
            return Err(bad("has a cargo rule with no spoken rule"));
        }
        if !cargo_rule && sized.iter().all(Option::is_none) {
            return Err(bad("models no rule"));
        }
        for text in [&self.hazmat_text, &self.size_text] {
            if text.contains('$') {
                return Err(bad("speaks money as a symbol"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crossing(hazmat: HazmatRule) -> RoadCrossing {
        RoadCrossing {
            kind: "tunnel".into(),
            name: "Test Tunnel".into(),
            hazmat,
            bulk_flammable_banned: false,
            tank_banned: false,
            regulated_lb_limit: None,
            max_length_ft: None,
            max_width_ft: None,
            max_height_ft: None,
            hazmat_text: "No hazardous materials.".into(),
            size_text: String::new(),
            source_url: "https://example.gov/rule".into(),
            access_date: "2026-10-11".into(),
            basis: "read".into(),
        }
    }

    fn rig() -> CrossingLoad {
        CrossingLoad::plain(73.5, 8.5, 13.5)
    }

    #[test]
    fn all_placarded_turns_away_both_placarded_loads_only() {
        let c = crossing(HazmatRule::All);
        assert!(c
            .refusal(&rig().with_cargo("hazardous", false, 20_000))
            .is_some());
        assert!(c
            .refusal(&rig().with_cargo("fuel_bulk", true, 40_000))
            .is_some());
        assert!(c
            .refusal(&rig().with_cargo("chemicals", false, 20_000))
            .is_none());
        assert!(c
            .refusal(&rig().with_cargo("general", false, 20_000))
            .is_none());
    }

    #[test]
    fn class_bans_refuse_packaged_mix_and_bulk_fuel_only_when_named() {
        let mut c = crossing(HazmatRule::Classes);
        assert!(c
            .refusal(&rig().with_cargo("hazardous", false, 20_000))
            .is_some());
        assert!(c
            .refusal(&rig().with_cargo("fuel_bulk", true, 40_000))
            .is_none());
        c.bulk_flammable_banned = true;
        assert!(c
            .refusal(&rig().with_cargo("fuel_bulk", true, 40_000))
            .is_some());
    }

    #[test]
    fn tank_regulated_weight_and_size_rules() {
        let mut c = crossing(HazmatRule::None);
        c.tank_banned = true;
        assert!(c
            .refusal(&rig().with_cargo("liquid_food", true, 40_000))
            .is_some());
        c.tank_banned = false;
        c.regulated_lb_limit = Some(400);
        assert!(c
            .refusal(&rig().with_cargo("chemicals", false, 401))
            .is_some());
        assert!(c
            .refusal(&rig().with_cargo("chemicals", false, 400))
            .is_none());
        c.max_height_ft = Some(13.5);
        c.size_text = "Too tall.".into();
        assert!(c.refusal(&rig()).is_none());
        assert_eq!(
            c.refusal(&CrossingLoad::plain(73.5, 8.5, 14.0)),
            Some("Too tall.")
        );
    }

    #[test]
    fn validation_requires_a_source_and_a_spoken_rule() {
        let mut c = crossing(HazmatRule::All);
        assert!(c.validate("t").is_ok());
        c.access_date = "yesterday".into();
        assert!(c.validate("t").is_err());
        let mut c = crossing(HazmatRule::All);
        c.hazmat_text.clear();
        assert!(c.validate("t").is_err());
        let mut c = crossing(HazmatRule::None);
        assert!(c.validate("t").is_err());
        c.max_height_ft = Some(13.5);
        assert!(c.validate("t").is_err());
        c.size_text = "Over 13 feet 6 inches needs a permit.".into();
        assert!(c.validate("t").is_ok());
    }
}
