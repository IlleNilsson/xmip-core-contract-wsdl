//! A WSDL document read into what it defines: messages, port types or
//! interfaces, bindings, services and their ports or endpoints — and the
//! references between them, which is where a description is unsound.

use roxmltree::{Document, Node};

pub const WSDL_11: &str = "http://schemas.xmlsoap.org/wsdl/";
pub const WSDL_20: &str = "http://www.w3.org/ns/wsdl";

/// Which WSDL the document is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    /// `definitions` in the 2001 namespace.
    V11,
    /// `description` in the W3C namespace.
    V20,
}

/// What a description defines, by local name, and what refers to what.
#[derive(Clone, Debug, Default)]
pub struct Definitions {
    pub version: Option<Version>,
    pub messages: Vec<String>,
    pub port_types: Vec<String>,
    pub bindings: Vec<String>,
    pub services: Vec<String>,
    /// Service and port or endpoint name.
    pub ports: Vec<(String, String)>,
    /// Every reference: what kind of thing refers, its name, what it refers
    /// to and the referred kind, in document order.
    pub references: Vec<Reference>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub from: String,
    pub to_kind: &'static str,
    pub to: String,
}

impl Definitions {
    /// Read `text` as a WSDL description.
    ///
    /// # Errors
    /// Not well-formed XML, or a root that is neither `definitions` nor
    /// `description` in a WSDL namespace.
    pub fn parse(text: &str) -> Result<Self, String> {
        let document = Document::parse(text).map_err(|error| format!("not XML: {error}"))?;
        let root = document.root_element();
        let version = match (root.tag_name().namespace(), root.tag_name().name()) {
            (Some(WSDL_11), "definitions") => Version::V11,
            (Some(WSDL_20), "description") => Version::V20,
            (namespace, name) => {
                return Err(format!(
                    "the root is {name} in {}, not a WSDL description",
                    namespace.unwrap_or("no namespace")
                ));
            }
        };
        let mut definitions = Self {
            version: Some(version),
            ..Self::default()
        };
        for child in root.children().filter(Node::is_element) {
            definitions.read(version, child);
        }
        Ok(definitions)
    }

    fn read(&mut self, version: Version, node: Node) {
        let name = attribute(node, "name");
        match (version, node.tag_name().name()) {
            (Version::V11, "message") => self.messages.push(name),
            (Version::V11, "portType") | (Version::V20, "interface") => {
                self.port_types.push(name.clone());
                for operation in named(node, "operation") {
                    let from = format!("operation {name}/{}", attribute(operation, "name"));
                    for part in operation.children().filter(Node::is_element) {
                        let referred = if version == Version::V11 {
                            attribute(part, "message")
                        } else {
                            attribute(part, "element")
                        };
                        let kind = if version == Version::V11 {
                            "message"
                        } else {
                            "element"
                        };
                        if !referred.is_empty() && kind == "message" {
                            self.refer(from.clone(), kind, local(&referred));
                        }
                    }
                }
            }
            (_, "binding") => {
                self.bindings.push(name.clone());
                let referred = if version == Version::V11 {
                    attribute(node, "type")
                } else {
                    attribute(node, "interface")
                };
                if !referred.is_empty() {
                    self.refer(format!("binding {name}"), "portType", local(&referred));
                }
            }
            (_, "service") => {
                self.services.push(name.clone());
                let port_name = if version == Version::V11 {
                    "port"
                } else {
                    "endpoint"
                };
                for port in named(node, port_name) {
                    let port_name = attribute(port, "name");
                    self.ports.push((name.clone(), port_name.clone()));
                    let binding = attribute(port, "binding");
                    self.refer(
                        format!("port {name}/{port_name}"),
                        "binding",
                        local(&binding),
                    );
                }
            }
            _ => {}
        }
    }

    fn refer(&mut self, from: String, to_kind: &'static str, to: String) {
        self.references.push(Reference { from, to_kind, to });
    }

    /// Every reference to something the description does not define.
    #[must_use]
    pub fn dangling(&self) -> Vec<&Reference> {
        self.references
            .iter()
            .filter(|reference| {
                let defined = match reference.to_kind {
                    "message" => &self.messages,
                    "portType" => &self.port_types,
                    "binding" => &self.bindings,
                    _ => return false,
                };
                !defined.contains(&reference.to)
            })
            .collect()
    }
}

fn attribute(node: Node, name: &str) -> String {
    node.attribute(name).unwrap_or("").to_string()
}

fn named<'a>(node: Node<'a, 'a>, name: &'a str) -> impl Iterator<Item = Node<'a, 'a>> {
    node.children()
        .filter(move |child| child.is_element() && child.tag_name().name() == name)
}

/// `tns:Order` as `Order`.
fn local(qualified: &str) -> String {
    qualified
        .rsplit(':')
        .next()
        .unwrap_or(qualified)
        .to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const ORDERS_11: &str = r#"<?xml version="1.0"?>
<definitions name="Orders" targetNamespace="urn:orders" xmlns:tns="urn:orders"
    xmlns="http://schemas.xmlsoap.org/wsdl/"
    xmlns:soap="http://schemas.xmlsoap.org/wsdl/soap/">
  <message name="PlaceOrderIn"><part name="body" element="tns:Order"/></message>
  <message name="PlaceOrderOut"><part name="body" element="tns:Receipt"/></message>
  <portType name="OrdersPortType">
    <operation name="PlaceOrder">
      <input message="tns:PlaceOrderIn"/>
      <output message="tns:PlaceOrderOut"/>
    </operation>
  </portType>
  <binding name="OrdersSoap" type="tns:OrdersPortType">
    <soap:binding style="document" transport="http://schemas.xmlsoap.org/soap/http"/>
  </binding>
  <service name="OrderService">
    <port name="OrdersPort" binding="tns:OrdersSoap">
      <soap:address location="http://example/orders"/>
    </port>
  </service>
</definitions>"#;

    pub const ORDERS_20: &str = r#"<description xmlns="http://www.w3.org/ns/wsdl"
    targetNamespace="urn:orders" xmlns:tns="urn:orders">
  <interface name="Orders"><operation name="place"/></interface>
  <binding name="OrdersHttp" interface="tns:Orders" type="http://www.w3.org/ns/wsdl/http"/>
  <service name="OrderService" interface="tns:Orders">
    <endpoint name="Main" binding="tns:OrdersHttp" address="http://example/orders"/>
  </service>
</description>"#;

    #[test]
    fn both_versions_read_into_their_definitions_and_references() {
        let v11 = Definitions::parse(ORDERS_11).expect("1.1");
        assert_eq!(v11.version, Some(Version::V11));
        assert_eq!(v11.messages, ["PlaceOrderIn", "PlaceOrderOut"]);
        assert_eq!(v11.port_types, ["OrdersPortType"]);
        assert_eq!(v11.bindings, ["OrdersSoap"]);
        assert_eq!(v11.services, ["OrderService"]);
        assert_eq!(
            v11.ports,
            [("OrderService".to_string(), "OrdersPort".to_string())]
        );
        assert_eq!(v11.references.len(), 4);
        assert!(v11.dangling().is_empty());
        let v20 = Definitions::parse(ORDERS_20).expect("2.0");
        assert_eq!(v20.version, Some(Version::V20));
        assert_eq!(v20.port_types, ["Orders"]);
        assert_eq!(
            v20.ports,
            [("OrderService".to_string(), "Main".to_string())]
        );
        assert!(v20.dangling().is_empty());
    }

    #[test]
    fn a_dangling_reference_is_named_and_a_non_wsdl_root_is_refused() {
        let broken = ORDERS_11
            .replace("type=\"tns:OrdersPortType\"", "type=\"tns:Missing\"")
            .replace("message=\"tns:PlaceOrderOut\"", "message=\"tns:Gone\"");
        let definitions = Definitions::parse(&broken).expect("parse");
        let dangling = definitions.dangling();
        assert_eq!(dangling.len(), 2);
        assert_eq!(dangling[0].from, "operation OrdersPortType/PlaceOrder");
        assert_eq!(dangling[0].to, "Gone");
        assert_eq!(dangling[1].from, "binding OrdersSoap");
        assert_eq!(dangling[1].to, "Missing");
        assert!(Definitions::parse("<order/>").is_err());
        assert!(Definitions::parse("<definitions xmlns=\"urn:x\"/>").is_err());
        assert!(Definitions::parse("<definitions").is_err());
    }
}
