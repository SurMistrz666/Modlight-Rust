//! mammoth `lib/html/simplify.js`: empty elements removed (unless void or
//! force-written), then adjacent non-fresh elements with the same tag and
//! attributes merged.

use super::ast::{HtmlNode, is_void_element};

/// `simplify(nodes)`.
pub fn simplify(nodes: Vec<HtmlNode>) -> Vec<HtmlNode> {
    collapse(remove_empty(nodes))
}

fn collapse(nodes: Vec<HtmlNode>) -> Vec<HtmlNode> {
    let mut children = Vec::new();
    for node in nodes {
        let collapsed = match node {
            HtmlNode::Element { tag, children } => HtmlNode::Element { tag, children: collapse(children) },
            other => other,
        };
        append_child(&mut children, collapsed);
    }
    children
}

fn append_child(children: &mut Vec<HtmlNode>, child: HtmlNode) {
    let mergeable = match (&child, children.last()) {
        (HtmlNode::Element { tag, .. }, Some(HtmlNode::Element { tag: last_tag, .. })) => {
            !tag.fresh && tag.matches_element(last_tag)
        }
        _ => false,
    };
    if !mergeable {
        children.push(child);
        return;
    }
    let HtmlNode::Element { tag, children: grand_children } = child else { unreachable!("checked above") };
    let Some(HtmlNode::Element { children: last_children, .. }) = children.last_mut() else {
        unreachable!("checked above")
    };
    if let Some(separator) = &tag.separator {
        append_child(last_children, HtmlNode::Text(separator.clone()));
    }
    for grand_child in grand_children {
        // Mutation is fine since simplifying elements create a copy of the children.
        append_child(last_children, grand_child);
    }
}

fn remove_empty(nodes: Vec<HtmlNode>) -> Vec<HtmlNode> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            HtmlNode::Element { tag, children } => {
                // isVoidElement looks at the element before its children were
                // emptied.
                let void = is_void_element(&tag, &children);
                let children = remove_empty(children);
                if !children.is_empty() || void {
                    out.push(HtmlNode::Element { tag, children });
                }
            }
            HtmlNode::Text(value) => {
                if !value.is_empty() {
                    out.push(HtmlNode::Text(value));
                }
            }
            other => out.push(other),
        }
    }
    out
}
