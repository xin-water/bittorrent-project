use std::time::Duration;

#[derive(Clone)]
pub struct HandshakeTimer {
    duration: Duration,
}

impl HandshakeTimer {
    pub fn new(duration: Duration) -> HandshakeTimer {
        HandshakeTimer {
            duration: duration,
        }
    }

    pub fn timeout(&self) {
        std::thread::sleep(self.duration);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::HandshakeTimer;

    #[test]
    fn positive_finish_before_timeout() {
        let timer = HandshakeTimer::new(Duration::from_millis(50));
    }

    #[test]
    #[should_panic]
    fn negative_finish_after_timeout() {
        let timer = HandshakeTimer::new(Duration::from_millis(50));
    }
}
