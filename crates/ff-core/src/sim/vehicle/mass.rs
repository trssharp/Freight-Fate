//! Fuel-inclusive operating weight. No extra state is stored in saves.

use super::{TruckState, KG_PER_LB, LEGAL_GVW_KG, REFERENCE_CARGO_KG, TRAILER_TARE_KG};

/// Assumed diesel density: 7.1 pounds per US gallon. This fixed calibration
/// omits changes with fuel blend and temperature; it is not a measured value.
pub const DIESEL_KG_PER_GAL: f64 = 7.1 * KG_PER_LB;

impl TruckState {
    /// Tractor and attached empty trailer, excluding diesel.
    /// Rated mass is calibrated at reference cargo and a full tank, so its
    /// fuel allowance is removed before the remaining fuel is added back.
    pub fn dry_tare_kg(&self) -> f64 {
        let combination = (self.specs.mass_kg
            - REFERENCE_CARGO_KG
            - self.specs.fuel_tank_gal.max(0.0) * DIESEL_KG_PER_GAL)
            .max(0.0);
        if self.trailer_attached {
            combination
        } else {
            (combination - TRAILER_TARE_KG).max(0.0)
        }
    }

    /// Diesel mass for a tank reading, capped at this truck's tank capacity.
    pub fn fuel_mass_for_gallons_kg(&self, fuel_gal: f64) -> f64 {
        fuel_gal.clamp(0.0, self.specs.fuel_tank_gal.max(0.0)) * DIESEL_KG_PER_GAL
    }

    pub fn fuel_mass_kg(&self) -> f64 {
        self.fuel_mass_for_gallons_kg(self.fuel_gal)
    }

    /// Remaining legal gross-weight capacity; negative means overweight.
    pub fn gross_weight_margin_kg(&self) -> f64 {
        LEGAL_GVW_KG - self.gross_mass_kg()
    }

    /// Legal capacity after replacing the current payload with `cargo_kg`.
    pub fn gross_weight_margin_with_cargo_kg(&self, cargo_kg: f64) -> f64 {
        LEGAL_GVW_KG - (self.tare_kg() + cargo_kg.max(0.0))
    }

    /// Gross weight after changing only the fuel quantity.
    pub fn gross_mass_after_fuel_kg(&self, fuel_gal: f64) -> f64 {
        self.dry_tare_kg() + self.fuel_mass_for_gallons_kg(fuel_gal) + self.cargo_kg.max(0.0)
    }

    /// Legal capacity after changing only the fuel quantity.
    pub fn gross_weight_margin_after_fuel_kg(&self, fuel_gal: f64) -> f64 {
        LEGAL_GVW_KG - self.gross_mass_after_fuel_kg(fuel_gal)
    }
}
