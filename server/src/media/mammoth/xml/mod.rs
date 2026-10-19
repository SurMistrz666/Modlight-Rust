//! mammoth `lib/xml/`: reading XML parts into a small element tree, over a
//! port of @xmldom/xmldom 0.8.13 ([`xmldom`]).

pub mod nodes;
pub mod reader;
pub mod xmldom;

pub use nodes::{EMPTY_ELEMENT, XmlElement, XmlNode};
