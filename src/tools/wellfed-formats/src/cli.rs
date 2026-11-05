//! The device model as every bench tool takes it: `--device plaf103`.
//!
//! `wellfed_proto::device::Model` is `no_std` and takes no clap, so the
//! flag is a newtype over it whose values are [`Model::ALL`] under their
//! [`Model::name`]s: a new model is a new value here with nothing to add.
//! There is no default: a tool names the device it works on.

use clap::ValueEnum;
use clap::builder::PossibleValue;
use wellfed_proto::device::Model;

/// A `--device` value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device(pub Model);

const ALL: [Device; Model::ALL.len()] = {
    let mut all = [Device(Model::ALL[0]); Model::ALL.len()];
    let mut i = 0;
    while i < all.len() {
        all[i] = Device(Model::ALL[i]);
        i += 1;
    }
    all
};

impl ValueEnum for Device {
    fn value_variants<'a>() -> &'a [Self] {
        &ALL
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(PossibleValue::new(self.0.name()))
    }
}

impl From<Device> for Model {
    fn from(device: Device) -> Self {
        device.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_is_a_value_under_its_name() {
        for model in Model::ALL {
            assert_eq!(Device::from_str(model.name(), false), Ok(Device(model)));
        }
        assert!(Device::from_str("feeder", false).is_err());
    }
}
