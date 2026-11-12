// Generated from wellfed-model by src/app/rust/tests/protocol.rs; do not edit.

/// The feeder's limits: it refuses a value outside them, so a control never
/// offers one.
abstract final class Limits {
  /// The fewest portions one feed or meal asks for.
  static const portionsLeast = 1;

  /// The most portions one feed or meal asks for.
  static const portionsMost = 48;

  /// The meals a feeder keeps.
  static const meals = 10;

  /// The most times one meal plays the meal call.
  static const callPlaysMost = 5;

  /// The most keys a feeder lists: owners, members and the server.
  static const clientsMost = 8;

  /// The lightest portion weight the feeder stores, in milligrams.
  static const portionMgLeast = 1000;

  /// The heaviest portion weight the feeder stores, in milligrams.
  static const portionMgMost = 50000;

  /// The shortest desiccant reminder period the feeder takes, in days.
  static const desiccantDaysLeast = 1;

  /// The longest desiccant reminder period the feeder takes, in days.
  static const desiccantDaysMost = 90;
}
