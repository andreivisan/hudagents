use std::time::Duration;

pub const TEMPERATURE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum TemperatureError {
    #[error("Failed to read Pi temperature: {0}")]
    Read(#[from] std::io::Error),

    #[error("Pi temperature is not a valid integer in millidegrees Celsius: {0}")]
    InvalidNumber(#[from] std::num::ParseIntError),

    #[error(
        "Pi temperature {value} millidegrees Celsius is outside the supported range -40000..=150000"
    )]
    OutOfRange { value: i32 },

    #[error("Reading Pi temperature timed out after 1 second")]
    Timeout,
}

pub fn parse_temperature(raw: &str) -> Result<i32, TemperatureError> {
    let trimmed = raw.trim();
    let parsed = trimmed.parse::<i32>()?;
    match parsed {
        -40_000..=150_000 => Ok(parsed),
        _ => Err(TemperatureError::OutOfRange { value: parsed }),
    }
}

pub async fn read_temperature() -> Result<i32, TemperatureError> {
    match tokio::time::timeout(
        TEMPERATURE_TIMEOUT,
        tokio::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp"),
    )
    .await
    {
        Ok(Ok(temperature)) => parse_temperature(&temperature),
        Ok(Err(error)) => Err(TemperatureError::Read(error)),
        Err(_) => Err(TemperatureError::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sensor_readings_and_inclusive_temperature_bounds() {
        for (input, expected) in [
            ("53250\n", 53_250),
            (" \t42000\r\n", 42_000),
            ("-40000", -40_000),
            ("150000", 150_000),
            ("-1", -1),
            ("0", 0),
        ] {
            assert_eq!(parse_temperature(input).unwrap(), expected, "{input:?}");
        }
    }

    #[test]
    fn rejects_readings_outside_temperature_bounds() {
        for value in [-40_001, 150_001, i32::MIN, i32::MAX] {
            let error = parse_temperature(&value.to_string()).unwrap_err();
            assert!(
                matches!(error, TemperatureError::OutOfRange { value: actual } if actual == value)
            );
            assert_eq!(
                error.to_string(),
                format!(
                    "Pi temperature {value} millidegrees Celsius is outside the supported range -40000..=150000"
                )
            );
        }
    }

    #[test]
    fn rejects_empty_noninteger_and_overflowing_readings() {
        for input in [
            "",
            " \n\t",
            "hot",
            "53.25",
            "53250.0",
            "53250 C",
            "1\n2",
            "2147483648",
            "-2147483649",
        ] {
            let error = parse_temperature(input).unwrap_err();
            assert!(
                matches!(error, TemperatureError::InvalidNumber(_)),
                "{input:?}"
            );
            assert!(
                error
                    .to_string()
                    .starts_with("Pi temperature is not a valid integer in millidegrees Celsius: ")
            );
        }
    }

    #[test]
    fn read_and_timeout_errors_explain_the_failure() {
        let error = TemperatureError::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "sensor missing",
        ));
        assert_eq!(
            error.to_string(),
            "Failed to read Pi temperature: sensor missing"
        );
        assert_eq!(
            TemperatureError::Timeout.to_string(),
            "Reading Pi temperature timed out after 1 second"
        );
    }
}
