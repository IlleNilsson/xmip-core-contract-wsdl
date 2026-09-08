#![forbid(unsafe_code)]

//! The WSDL content contract — a technology of `xmip-core-contract`.
//!
//! Two claims, decided 2026-09-07 (ADR-0042): **well-formedness is a given**
//! and **conformance is a given once a contract is named**.
//!
//! Well-formed here is a *sound description*: well-formed XML whose root is
//! WSDL 1.1's `definitions` or WSDL 2.0's `description`, and in which every
//! reference lands — a binding on a port type or interface it names, a port
//! or endpoint on a binding, an operation on its messages. A description
//! that refers to what it does not define is the one that fails at the
//! first call, and it is caught here instead.
//!
//! Conformance is the *service*: a Location that names this contract with
//! `OrderService` bound has every description held to defining that
//! service, and `OrderService/OrdersPort` to that port or endpoint of it.
//! The XML Schema the messages are typed by is `xmip-core-contract-xml-schema`,
//! and holding a description's `types` section to it is the next layer here.

pub mod definitions;

use contract::{
    Contract, ContractDescriptor, ContractError, ContractFactory, ContractId, ValidationIssue,
    ValidationResult,
};
use definitions::Definitions;
use stream::Stream;

/// The bound service, and optionally its port or endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub port: Option<String>,
}

impl Service {
    /// `OrderService` or `OrderService/OrdersPort`.
    ///
    /// # Errors
    /// An empty name, or more than one slash.
    pub fn parse(reference: &str) -> Result<Self, ContractError> {
        let parts: Vec<&str> = reference.split('/').map(str::trim).collect();
        match parts.as_slice() {
            [name] if !name.is_empty() => Ok(Self {
                name: (*name).to_string(),
                port: None,
            }),
            [name, port] if !name.is_empty() && !port.is_empty() => Ok(Self {
                name: (*name).to_string(),
                port: Some((*port).to_string()),
            }),
            _ => Err(ContractError {
                message: format!("{reference:?} is not SERVICE or SERVICE/PORT"),
            }),
        }
    }

    fn reference(&self) -> String {
        match &self.port {
            Some(port) => format!("{}/{port}", self.name),
            None => self.name.clone(),
        }
    }
}

/// The WSDL contract, bare or bound to a service.
pub struct Wsdl {
    descriptor: ContractDescriptor,
    service: Option<Service>,
}

impl Wsdl {
    /// A sound description, of any services.
    #[must_use]
    pub fn new() -> Self {
        Self {
            descriptor: descriptor("wsdl"),
            service: None,
        }
    }

    /// A sound description that defines `service`.
    #[must_use]
    pub fn of(service: Service) -> Self {
        Self {
            descriptor: descriptor(&format!("wsdl:{}", service.reference())),
            service: Some(service),
        }
    }

    /// Whether a service is bound.
    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.service.is_some()
    }
}

impl Default for Wsdl {
    fn default() -> Self {
        Self::new()
    }
}

fn descriptor(id: &str) -> ContractDescriptor {
    ContractDescriptor {
        id: ContractId(id.to_string()),
        version: "1".to_string(),
        representation: "application/wsdl+xml".to_string(),
    }
}

impl Contract for Wsdl {
    fn descriptor(&self) -> &ContractDescriptor {
        &self.descriptor
    }

    fn identify(&self, stream: &Stream) -> Result<bool, ContractError> {
        if stream.media_type().is_some_and(|m| {
            m.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/wsdl+xml")
        }) {
            return Ok(true);
        }
        let Ok(text) = std::str::from_utf8(stream.bytes()) else {
            return Ok(false);
        };
        Ok(text.contains(definitions::WSDL_11) || text.contains(definitions::WSDL_20))
    }

    fn validate(&self, stream: &Stream) -> Result<ValidationResult, ContractError> {
        let text = match std::str::from_utf8(stream.bytes()) {
            Ok(text) => text,
            Err(error) => {
                return Ok(result(vec![issue(
                    "malformed",
                    &format!("not text: {error}"),
                    None,
                )]));
            }
        };
        let definitions = match Definitions::parse(text) {
            Ok(definitions) => definitions,
            Err(message) => return Ok(result(vec![issue("malformed", &message, None)])),
        };
        let mut issues: Vec<ValidationIssue> = definitions
            .dangling()
            .into_iter()
            .map(|reference| {
                issue(
                    "reference",
                    &format!(
                        "refers to {} {}, which is not defined",
                        reference.to_kind, reference.to
                    ),
                    Some(reference.from.clone()),
                )
            })
            .collect();
        if let Some(service) = &self.service {
            if !definitions.services.contains(&service.name) {
                issues.push(issue(
                    "service",
                    &format!("does not define service {}", service.name),
                    None,
                ));
            } else if let Some(port) = &service.port
                && !definitions
                    .ports
                    .iter()
                    .any(|(s, p)| s == &service.name && p == port)
            {
                issues.push(issue(
                    "service",
                    &format!("service {} has no port {port}", service.name),
                    Some(format!("service {}", service.name)),
                ));
            }
        }
        Ok(result(issues))
    }
}

fn issue(code: &str, message: &str, path: Option<String>) -> ValidationIssue {
    ValidationIssue {
        code: code.to_string(),
        message: message.to_string(),
        path,
    }
}

fn result(issues: Vec<ValidationIssue>) -> ValidationResult {
    ValidationResult {
        valid: issues.is_empty(),
        issues,
    }
}

/// Loads the contract a Location names: an empty reference is the bare
/// contract, anything else a service, `OrderService` or
/// `OrderService/OrdersPort`.
pub struct WsdlFactory;

impl ContractFactory for WsdlFactory {
    fn technology(&self) -> &'static str {
        "wsdl"
    }

    fn load(&self, reference: &str) -> Result<Box<dyn Contract>, ContractError> {
        if reference.trim().is_empty() {
            return Ok(Box::new(Wsdl::new()));
        }
        Ok(Box::new(Wsdl::of(Service::parse(reference)?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::tests::{ORDERS_11, ORDERS_20};
    use xcore::StreamId;

    fn stream(text: &str, media_type: Option<&str>) -> Stream {
        Stream::new(
            StreamId::new(1),
            text.as_bytes().to_vec(),
            media_type.map(str::to_string),
        )
    }

    #[test]
    fn a_sound_description_holds_bare_and_bound() {
        let bare = Wsdl::new();
        assert!(bare.identify(&stream(ORDERS_11, None)).expect("identify"));
        assert!(bare.identify(&stream(ORDERS_20, None)).expect("identify"));
        assert!(
            bare.identify(&stream("x", Some("application/wsdl+xml; charset=utf-8")))
                .expect("identify")
        );
        assert!(!bare.identify(&stream("<order/>", None)).expect("identify"));
        assert!(
            bare.validate(&stream(ORDERS_11, None))
                .expect("validate")
                .valid
        );
        assert!(
            bare.validate(&stream(ORDERS_20, None))
                .expect("validate")
                .valid
        );
        let bound = WsdlFactory.load("OrderService/OrdersPort").expect("load");
        assert_eq!(bound.descriptor().id.0, "wsdl:OrderService/OrdersPort");
        assert!(
            bound
                .validate(&stream(ORDERS_11, None))
                .expect("validate")
                .valid
        );
        assert!(Wsdl::of(Service::parse("OrderService").expect("parse")).is_bound());
        assert!(
            !WsdlFactory
                .load(" ")
                .expect("bare")
                .descriptor()
                .id
                .0
                .contains(':')
        );
    }

    #[test]
    fn a_missing_service_port_or_reference_is_named() {
        let bound = Wsdl::of(Service::parse("Billing").expect("parse"));
        let result = bound.validate(&stream(ORDERS_11, None)).expect("validate");
        assert!(!result.valid);
        assert_eq!(result.issues[0].code, "service");
        assert_eq!(result.issues[0].message, "does not define service Billing");
        let bound = Wsdl::of(Service::parse("OrderService/Other").expect("parse"));
        let result = bound.validate(&stream(ORDERS_20, None)).expect("validate");
        assert_eq!(
            result.issues[0].message,
            "service OrderService has no port Other"
        );
        let dangling = ORDERS_11.replace("binding=\"tns:OrdersSoap\"", "binding=\"tns:Nope\"");
        let result = Wsdl::new()
            .validate(&stream(&dangling, None))
            .expect("validate");
        assert_eq!(result.issues[0].code, "reference");
        assert_eq!(
            result.issues[0].path.as_deref(),
            Some("port OrderService/OrdersPort")
        );
        assert!(result.issues[0].message.contains("binding Nope"));
    }

    #[test]
    fn what_is_not_wsdl_does_not_hold() {
        let result = Wsdl::new()
            .validate(&stream("<definitions", None))
            .expect("validate");
        assert_eq!(result.issues[0].code, "malformed");
        let binary = Stream::new(StreamId::new(1), vec![0xff], None);
        assert!(!Wsdl::new().validate(&binary).expect("validate").valid);
        assert!(Service::parse("").is_err());
        assert!(Service::parse("a/b/c").is_err());
        assert!(Service::parse("a/").is_err());
    }
}
