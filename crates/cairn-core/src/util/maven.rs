//! Parses Maven coordinates of the form `group:artifact:version[:classifier][@ext]`.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MavenCoord {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: String,
}

impl MavenCoord {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let (body, extension) = match s.split_once('@') {
            Some((b, e)) => (b, e.to_string()),
            None => (s, "jar".to_string()),
        };
        let mut parts = body.split(':');
        let group = parts.next()?.to_string();
        let artifact = parts.next()?.to_string();
        let version = parts.next()?.to_string();
        let classifier = parts.next().map(str::to_string);
        if group.is_empty() || artifact.is_empty() || version.is_empty() || parts.next().is_some() {
            return None;
        }
        Some(Self { group, artifact, version, classifier, extension })
    }

    /// Path relative to `libraries/`, always using `/` as the separator.
    pub fn path(&self) -> String {
        let file = match &self.classifier {
            Some(c) => format!("{}-{}-{}.{}", self.artifact, self.version, c, self.extension),
            None => format!("{}-{}.{}", self.artifact, self.version, self.extension),
        };
        format!("{}/{}/{}/{}", self.group.replace('.', "/"), self.artifact, self.version, file)
    }

    /// Key for de-duplication. It does not include the version.
    pub fn key(&self) -> String {
        match &self.classifier {
            Some(c) => format!("{}:{}:{}", self.group, self.artifact, c),
            None => format!("{}:{}", self.group, self.artifact),
        }
    }

    pub fn with_classifier(&self, classifier: &str) -> Self {
        Self { classifier: Some(classifier.to_string()), ..self.clone() }
    }
}

impl fmt::Display for MavenCoord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.group, self.artifact, self.version)?;
        if let Some(c) = &self.classifier {
            write!(f, ":{c}")?;
        }
        if self.extension != "jar" {
            write!(f, "@{}", self.extension)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_coordinate() {
        let c = MavenCoord::parse("net.minecraftforge:forge:1.20.1-47.2.0:client@zip").unwrap();
        assert_eq!(c.path(), "net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0-client.zip");
        assert_eq!(c.key(), "net.minecraftforge:forge:client");
    }

    #[test]
    fn rejects_short_coordinate() {
        assert!(MavenCoord::parse("a:b").is_none());
    }
}
