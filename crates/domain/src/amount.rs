use std::{fmt, str::FromStr};

use num_bigint::BigUint;

macro_rules! non_negative_amount {
    ($name:ident) => {
        #[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(BigUint);

        impl $name {
            #[must_use]
            pub fn zero() -> Self {
                Self(BigUint::default())
            }

            #[must_use]
            pub fn from_biguint(value: BigUint) -> Self {
                Self(value)
            }

            #[must_use]
            pub fn as_biguint(&self) -> &BigUint {
                &self.0
            }

            #[must_use]
            pub fn into_biguint(self) -> BigUint {
                self.0
            }

            #[must_use]
            pub fn is_zero(&self) -> bool {
                self.0 == BigUint::default()
            }

            #[must_use]
            pub fn to_decimal_string(&self) -> String {
                self.0.to_str_radix(10)
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self(BigUint::from(value))
            }
        }

        impl From<BigUint> for $name {
            fn from(value: BigUint) -> Self {
                Self(value)
            }
        }

        impl FromStr for $name {
            type Err = num_bigint::ParseBigIntError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                BigUint::from_str(value).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

non_negative_amount!(Coins);
non_negative_amount!(EntityCount);
non_negative_amount!(StockCent);
non_negative_amount!(GrowthCent);
non_negative_amount!(LifetimePurchaseCount);
