// Copyright (C) 2026 Toit contributors.

//! How the settings files are layered: the keys an agent sets stand at the
//! top level of the router, placer and layout settings; the tuning of the
//! machinery stands in one nested group of each, `tuning`
//! (docs/architecture.md, "Settings").
//!
//! - [`tuning_group!`] declares a tuning group whose `core` section mirrors
//!   a core config's own fields by name and type, applied to it in one step.
//! - [`settings_with_moved_keys!`] gives a settings struct its serde
//!   implementations: a key that moved into the group is refused with
//!   where it lives now (`"x" moved to "tuning.x"`), everything else is
//!   read as the struct's derive reads it.
//! - [`field_names`] lists the keys a derived struct reads.

/// The name of the nested group in every settings file.
pub const TUNING: &str = "tuning";

/// The keys a struct whose `Deserialize` is derived reads (serde's own
/// list, as its unknown-field errors give it).
pub fn field_names<'de, T: serde::Deserialize<'de>>() -> &'static [&'static str] {
    let mut names: &'static [&'static str] = &[];
    let _ = T::deserialize(FieldNames(&mut names));
    names
}

/// A deserializer that records the field list a derived struct asks for
/// and reads nothing.
pub struct FieldNames<'a>(pub &'a mut &'static [&'static str]);

/// The error that ends [`FieldNames`]' reading.
#[derive(Debug)]
pub struct Recorded;

impl std::fmt::Display for Recorded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("field names recorded")
    }
}

impl std::error::Error for Recorded {}

impl serde::de::Error for Recorded {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Recorded
    }
}

impl<'de> serde::Deserializer<'de> for FieldNames<'_> {
    type Error = Recorded;

    fn deserialize_any<V: serde::de::Visitor<'de>>(self, _: V) -> Result<V::Value, Recorded> {
        Err(Recorded)
    }

    fn deserialize_struct<V: serde::de::Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Recorded> {
        *self.0 = fields;
        Err(Recorded)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

/// Where a top-level key of `settings` that moved into the tuning group
/// lives now: `renamed` maps an old name to its name in the group (the core
/// config's), any other key among `tuning_keys` kept its name.
pub fn moved_key(key: &str, tuning_keys: &[&str], renamed: &[(&str, &str)]) -> Option<String> {
    renamed
        .iter()
        .find(|(old, _)| *old == key)
        .map(|(_, new)| format!("{TUNING}.{new}"))
        .or_else(|| {
            tuning_keys
                .contains(&key)
                .then(|| format!("{TUNING}.{key}"))
        })
}

/// Refuses settings (`what`: "router", "placer", "layout") that name a
/// moved key at the top level, saying where it lives now.
pub fn refuse_moved_keys(
    value: &serde_json::Value,
    what: &str,
    tuning_keys: &[&str],
    renamed: &[(&str, &str)],
) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    for key in object.keys() {
        if let Some(new) = moved_key(key, tuning_keys, renamed) {
            return Err(format!(
                "the {what} setting \"{key}\" moved to \"{new}\" (the tuning group; docs/config-reference.md)"
            ));
        }
    }
    Ok(())
}

/// Values a tuning key's check tries in turn: the first that the key reads
/// and that differs from its default is its sample (`check_tuning_group`).
const SAMPLES: &str = r#"[7, 0.75, true, false, "unfinished", [0.25], {"U1": 0.5}, {},
    null, {"crossing": 2.0}, {"seed": 5}]"#;

/// The checks of a settings struct `T` with a tuning group (its tests in
/// the router, placer and layout crates call it): every moved key at the
/// top level is refused with the message naming its new place; every key
/// of the group reads a value other than its default inside `tuning` and
/// writes it back as read (a round trip); an unknown key inside the group
/// is refused; no key is both a top-level key and one of the group's.
/// Returns the moved keys checked, each with its sample in the group.
pub fn check_tuning_group<T>(
    what: &str,
    top_level_keys: &[&str],
    tuning_keys: &[&str],
    renamed: &[(&str, &str)],
) -> Result<Vec<(String, serde_json::Value)>, String>
where
    T: serde::de::DeserializeOwned + serde::Serialize + Default,
{
    use serde_json::{Value, json};
    let write = |config: &T| serde_json::to_value(config).map_err(|error| error.to_string());
    let default = write(&T::default())?;
    let samples: Vec<Value> = serde_json::from_str(SAMPLES).unwrap();
    for key in tuning_keys {
        if top_level_keys.contains(key) {
            return Err(format!("{key:?} is a top-level key and a tuning key"));
        }
    }
    for (old, new) in renamed {
        if top_level_keys.contains(old) || !tuning_keys.contains(new) {
            return Err(format!("{old:?} -> {new:?} is no rename into the group"));
        }
    }
    let mut checked = Vec::new();
    let old_names = renamed.iter().map(|(old, new)| (*old, *new)).chain(
        tuning_keys
            .iter()
            .filter(|key| !renamed.iter().any(|(_, new)| new == *key))
            .map(|key| (*key, *key)),
    );
    for (old, new) in old_names {
        let sample = samples
            .iter()
            .find_map(|sample| {
                let config: T = serde_json::from_value(json!({TUNING: {new: sample}})).ok()?;
                let written = write(&config).ok()?;
                (written[TUNING][new] != default[TUNING][new]).then_some((sample, written))
            })
            .ok_or_else(|| format!("tuning.{new}: no sample reads as other than the default"))?;
        let (sample, written) = sample;
        let again = write(
            &serde_json::from_value::<T>(written.clone()).map_err(|error| {
                format!("tuning.{new}: what was written does not read back: {error}")
            })?,
        )?;
        if again != written {
            return Err(format!("tuning.{new}: {written} reads back as {again}"));
        }
        let error = match serde_json::from_value::<T>(json!({old: sample})) {
            Ok(_) => return Err(format!("{old:?} at the top level is read")),
            Err(error) => error.to_string(),
        };
        let expected = format!("\"{old}\" moved to \"{TUNING}.{new}\"");
        if !error.contains(&expected) || !error.contains(what) {
            return Err(format!("{old:?} at the top level: {error}"));
        }
        checked.push((old.to_string(), sample.clone()));
    }
    match serde_json::from_value::<T>(json!({TUNING: {"no_such_key": 1}})) {
        Ok(_) => Err("an unknown key in the group is read".into()),
        Err(error) if error.to_string().contains("unknown field `no_such_key`") => Ok(checked),
        Err(error) => Err(format!("an unknown key in the group: {error}")),
    }
}

/// Implements `Deserialize` and `Serialize` for a settings struct whose
/// derives carry `#[serde(remote = "Self")]` (they then define inherent
/// functions of those names): a top-level key that moved into the tuning
/// group `$tuning` (or is renamed by `$renamed` on the way) is refused with
/// where it lives now; anything else is read and written by the derives.
#[macro_export]
macro_rules! settings_with_moved_keys {
    ($type:ty, $what:literal, $tuning:ty, $renamed:expr) => {
        impl<'de> serde::Deserialize<'de> for $type {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                use serde::de::Error;
                let value = serde_json::Value::deserialize(deserializer)?;
                $crate::settings::refuse_moved_keys(
                    &value,
                    $what,
                    $crate::settings::field_names::<$tuning>(),
                    $renamed,
                )
                .map_err(D::Error::custom)?;
                <$type>::deserialize(value).map_err(D::Error::custom)
            }
        }

        impl serde::Serialize for $type {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                <$type>::serialize(self, serializer)
            }
        }
    };
}

/// Declares a tuning group: a struct with serde's derives and unknown keys
/// refused, whose `core { ... }` fields (each an `Option` of the core
/// config's field of the same name and type) override that field where
/// given, all of them in one step (`apply`). The fields after the section
/// are the KiCad layer's own and are written as in any struct.
///
/// ```ignore
/// tuning_group! {
///     /// The router's tuning.
///     pub struct KiCadRouterTuning for pcb_router::Config {
///         core {
///             /// A via's cost in millimetres of track (default 8).
///             pub via_cost: Option<f64>,
///         }
///         /// Seeds an attempt that leaves connections open is routed again.
///         #[serde(default)]
///         pub retry_seeds: Option<usize>,
///     }
/// }
/// ```
///
/// A new core setting the KiCad layer should expose is one line in the
/// `core` section; the compiler checks its name and type against the core.
#[macro_export]
macro_rules! tuning_group {
    (
        $(#[$meta:meta])*
        pub struct $name:ident for $core:path {
            core {
                $( $(#[$core_meta:meta])* pub $core_field:ident: Option<$core_type:ty>, )*
            }
            $( $(#[$field_meta:meta])* pub $field:ident: $field_type:ty, )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $(
                $(#[$core_meta])*
                #[serde(default)]
                pub $core_field: Option<$core_type>,
            )*
            $( $(#[$field_meta])* pub $field: $field_type, )*
        }

        impl $name {
            /// The keys of the `core` section: the core config's own fields.
            pub const CORE: &'static [&'static str] = &[$(stringify!($core_field)),*];

            /// Sets every core field this group gives (its `core` section);
            /// the others keep what `config` holds.
            pub fn apply(&self, config: &mut $core) {
                $(
                    if let Some(value) = &self.$core_field {
                        config.$core_field = value.clone();
                    }
                )*
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Example {
        alpha: f64,
        #[serde(rename = "beta_mm")]
        beta: Option<usize>,
    }

    #[test]
    fn field_names_are_serdes_own() {
        assert_eq!(field_names::<Example>(), ["alpha", "beta_mm"]);
    }

    #[test]
    fn a_moved_key_says_where_it_lives_now() {
        let keys = ["seed", "pitches"];
        let renamed = [("grid_pitches_mm", "pitches")];
        assert_eq!(
            moved_key("seed", &keys, &renamed).as_deref(),
            Some("tuning.seed")
        );
        assert_eq!(
            moved_key("grid_pitches_mm", &keys, &renamed).as_deref(),
            Some("tuning.pitches")
        );
        assert_eq!(moved_key("seeds", &keys, &renamed), None);
        let error = refuse_moved_keys(
            &serde_json::json!({"seeds": 2, "grid_pitches_mm": [0.1]}),
            "router",
            &keys,
            &renamed,
        )
        .unwrap_err();
        assert!(
            error.contains("\"grid_pitches_mm\" moved to \"tuning.pitches\""),
            "{error}"
        );
    }
}
