use crate::*;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

/*
issue: hard to hash these things to generate IDs when spitting them out
using the span info is a hack and apparently unreliable between two parse calls on the same tokens
wait... is span info a hack? *DEBUG* info is a hack... not necessarily *SPAN* info.
if we can trace the token back to the original we can use the start / byte index
nvmd all spans are zero

rsx! {                              <-- 0
    if thing {                      <-- 1
        for item in other {         <-- 2
            Component {
                blah {}             <-- 3
            }
        }
        for item in other {         <-- 4
            Component {
                blah {}             <-- 5
            }
        }
    }
}
*/

#[derive(Debug)]
pub struct CallBodyContext<'a> {
    pub contexts: Vec<DynamicContext<'a>>,
}

// impl<'a> CallBodyContext<'a> {
//     /// Take a callbody and then explore all its dynamic nodes for dynamic contexts.
//     /// Returns them in a consistent DFS order so the codegen makes some sense.
//     /// The ID of each template is its index in the context list, so the root contet will have ID=0
//     ///
//     /// "Hot literals" will indexed relative to the context they're found in - ie 0:0 would be the
//     /// first hot literal in the first template. These can show up in a number of places.
//     pub fn from_callbody<Ctx: HotReloadingContext>(callbody: &'a CallBody) -> Self {
//         let mut contexts = vec![];

//         // Create the root dynamic context stack and push the root context onto it
//         let mut stack = vec![];
//         stack.push(callbody.roots.as_slice());

//         // And then walk its dynamic nodes looking for subtemplates
//         // push those onto the stack, chewing it down
//         // Make sure to do this in such an order that we are progressing depth-first
//         // We could also do this rec
//         while let Some(roots) = stack.pop() {
//             let cx = DynamicContext::from_body::<Ctx>(roots);

//             // Walk these backwards so children can end up in front of the parents
//             for node in cx.dynamic_nodes.iter().rev() {
//                 match node {
//                     // Elements can't be dynamic nodes
//                     BodyNode::Element(_) => unreachable!(),

//                     // Dynamic text nodes and exprs contain no child templates to explore
//                     BodyNode::Text(_) => continue,
//                     BodyNode::RawExpr(_) => continue,

//                     // Components might have a template to explore - I think we might need to push empty ones
//                     BodyNode::Component(c) => stack.push(&c.children.roots.as_slice()),

//                     // For loops a single template to explore
//                     BodyNode::ForLoop(f) => stack.push(f.body.as_slice()),

//                     // If chains have multiple templates to explore based on the chain
//                     // make sure we do this in reverse, I think?
//                     BodyNode::IfChain(chain) => {
//                         let mut local_chain: Vec<&[BodyNode]> = vec![];
//                         todo!("");
//                         stack.extend(local_chain.iter());
//                     }
//                 }
//             }

//             contexts.push(cx);
//         }

//         CallBodyContext { contexts }
//     }
// }

/// An understanding of a set of Tokens that provides the mapping required for:
/// - hotreload diffing
/// - converting to a Template
/// - converting to Tokens
///
/// As we create the dynamic nodes, we want to keep track of them in a linear fashion
/// We'll use the size of the vecs to determine the index of the dynamic node in the final output
#[derive(Default, Debug, PartialEq)]
pub struct DynamicContext<'a> {
    pub dynamic_nodes: Vec<&'a BodyNode>,
    pub dynamic_attributes: Vec<Vec<&'a AttributeType>>,

    // todo: this will be *all* the reloadable items, just not ifmts
    // for now though, it's dynamic ifmts
    pub ifmts: Vec<&'a IfmtInput>,

    pub current_path: Vec<u8>,
    pub node_paths: Vec<Vec<u8>>,
    pub attr_paths: Vec<Vec<u8>>,

    // The current ID of the template relative to the callbody
    // The root template is ID=0 and then every template is assigned an ID in the order in which it
    // is discovered
    pub template_idx: usize,

    dynamic_idx: usize,
}

impl<'a> DynamicContext<'a> {
    pub fn from_body<Ctx: HotReloadingContext>(roots: &'a [BodyNode]) -> Self {
        let mut s = Self::default();
        s.populate_all_by_updating::<Ctx>(roots);
        s
    }

    /// Populate the dynamic context with our own roots
    ///
    /// This will call update_node on each root, attempting to build us a list of TemplateNodes that
    /// we can render out.
    ///
    /// These TemplateNodes are the same one used in Dioxus core! We just serialize them out and then
    /// they'll get picked up after codegen for compilation. Cool stuff.
    ///
    /// If updating fails (IE the root is a dynamic node that has changed), then we return None.
    pub fn fill<Ctx: HotReloadingContext>(
        &mut self,
        roots: &'a [BodyNode],
    ) -> Option<Vec<TemplateNode>> {
        // Create a list of new roots that we'll spit out
        let mut roots_ = Vec::new();

        // Populate the dynamic context with our own roots
        for (idx, root) in roots.iter().enumerate() {
            self.current_path.push(idx as u8);
            roots_.push(self.update_node::<Ctx>(root)?);
            self.current_path.pop();
        }

        Some(roots_)
    }

    /// Get the next dynamic index
    fn next_dynamic_idx(&mut self) -> usize {
        let idx = self.dynamic_idx;
        self.dynamic_idx += 1;
        idx
    }

    pub fn populate_all_by_updating<Ctx: HotReloadingContext>(
        &mut self,
        roots: &'a [BodyNode],
    ) -> Option<Vec<Vec<TemplateNode>>> {
        // Create a list of new roots that we'll spit out
        let mut roots_ = Vec::new();

        // Populate the dynamic context with our own roots
        for (idx, root) in roots.iter().enumerate() {
            self.current_path.push(idx as u8);
            roots_.push(self.update_node::<Ctx>(root)?);
            self.current_path.pop();
        }

        // Now walk the the all_templates nodes and run the same dynamic context algorithm on them
        // This should bubble up internal templates

        Some(vec![roots_])
    }

    /// Render a portion of an rsx callbody to a TemplateNode call
    ///
    /// We're assembling the templatenodes
    pub fn render_static_node(&mut self, root: &'a BodyNode) -> TokenStream2 {
        match root {
            BodyNode::Element(el) => self.render_static_element(el),

            BodyNode::Text(text) if text.input.is_static() => {
                let text = text.input.to_static().unwrap();
                quote! { dioxus_core::TemplateNode::Text { text: #text } }
            }

            BodyNode::ForLoop(for_loop) => self.render_for_loop(root, for_loop),

            BodyNode::RawExpr(_)
            | BodyNode::Text(_)
            | BodyNode::IfChain(_)
            | BodyNode::Component(_) => self.render_dynamic_node(root),
        }
    }

    /// Render a for loop to a token stream
    ///
    /// This is basically just rendering a dynamic node, but with some extra bookkepping to track the
    /// contents of the for loop in case we want to hot reload it
    fn render_for_loop(&mut self, root: &'a BodyNode, _for_loop: &ForLoop) -> TokenStream2 {
        self.render_dynamic_node(root)
    }

    fn render_dynamic_node(&mut self, root: &'a BodyNode) -> TokenStream2 {
        let ct = self.dynamic_nodes.len();
        self.dynamic_nodes.push(root);
        self.node_paths.push(self.current_path.clone());
        match root {
            BodyNode::Text(_) => quote! { dioxus_core::TemplateNode::DynamicText { id: #ct } },
            _ => quote! { dioxus_core::TemplateNode::Dynamic { id: #ct } },
        }
    }

    fn render_static_element(&mut self, el: &'a Element) -> TokenStream2 {
        let el_name = &el.name;
        let ns = |name| match el_name {
            ElementName::Ident(i) => quote! { dioxus_elements::#i::#name },
            ElementName::Custom(_) => quote! { None },
        };

        let static_attrs = el
            .merged_attributes
            .iter()
            .map(|attr| self.render_merged_attributes(attr, ns, el_name))
            .collect::<Vec<_>>();

        let children = el
            .children
            .iter()
            .enumerate()
            .map(|(idx, root)| self.render_children_nodes(idx, root))
            .collect::<Vec<_>>();

        let ns = ns(quote!(NAME_SPACE));
        let el_name = el_name.tag_name();

        quote! {
            dioxus_core::TemplateNode::Element {
                tag: #el_name,
                namespace: #ns,
                attrs: &[ #(#static_attrs)* ],
                children: &[ #(#children),* ],
            }
        }
    }

    pub fn render_children_nodes(&mut self, idx: usize, root: &'a BodyNode) -> TokenStream2 {
        self.current_path.push(idx as u8);
        let out = self.render_static_node(root);
        self.current_path.pop();
        out
    }

    /// Render the attributes of an element
    fn render_merged_attributes(
        &mut self,
        attr: &'a AttributeType,
        ns: impl Fn(TokenStream2) -> TokenStream2,
        el_name: &ElementName,
    ) -> TokenStream2 {
        // Rendering static attributes requires a bit more work than just a dynamic attrs
        match attr.as_static_str_literal() {
            // If it's static, we'll take this little optimization
            Some((name, value)) => Self::render_static_attr(value, name, ns, el_name),

            // Otherwise, we'll just render it as a dynamic attribute
            // This will also insert the attribute into the dynamic_attributes list to assemble the final template
            _ => self.render_dynamic_attr(attr),
        }
    }

    fn render_static_attr(
        value: &IfmtInput,
        name: &ElementAttrName,
        ns: impl Fn(TokenStream2) -> TokenStream2,
        el_name: &ElementName,
    ) -> TokenStream2 {
        let value = value.to_static().unwrap();

        let ns = match name {
            ElementAttrName::BuiltIn(name) => ns(quote!(#name.1)),
            ElementAttrName::Custom(_) => quote!(None),
        };

        let name = match (el_name, name) {
            (ElementName::Ident(_), ElementAttrName::BuiltIn(_)) => quote! { #el_name::#name.0 },
            _ => {
                //hmmmm I think we could just totokens this, but the to_string might be inserting quotes
                let as_string = name.to_string();
                quote! { #as_string }
            }
        };

        quote! {
            dioxus_core::TemplateAttribute::Static {
                name: #name,
                namespace: #ns,
                value: #value,

                // todo: we don't diff these so we never apply the volatile flag
                // volatile: dioxus_elements::#el_name::#name.2,
            },
        }
    }

    /// If the attr is dynamic, we save it to the tracked attributes list
    /// This will let us use this context at a later point in time to update the template
    fn render_dynamic_attr(&mut self, attr: &'a AttributeType) -> TokenStream2 {
        let ct = self.dynamic_attributes.len();

        self.dynamic_attributes.push(vec![attr]);
        self.attr_paths.push(self.current_path.clone());

        quote! { dioxus_core::TemplateAttribute::Dynamic { id: #ct }, }
    }

    #[cfg(feature = "hot_reload")]
    pub fn update_node<Ctx: HotReloadingContext>(
        &mut self,
        root: &'a BodyNode,
    ) -> Option<TemplateNode> {
        match root {
            // The user is moving a static node around in the template
            // Check this is a bit more complex but we can likely handle it
            BodyNode::Element(el) => self.update_element::<Ctx>(el),

            BodyNode::Text(text) if text.input.is_static() => {
                let text = text.input.source.as_ref().unwrap();
                let text = intern(text.value().as_str());
                Some(TemplateNode::Text { text })
            }

            // The user is moving a dynamic node around in the template
            // We *might* be able to handle it, but you never really know
            BodyNode::RawExpr(_)
            | BodyNode::ForLoop(_)
            | BodyNode::Text(_)
            | BodyNode::IfChain(_)
            | BodyNode::Component(_) => self.update_dynamic_node(root),
        }
    }

    /// Attempt to update a dynamic node in the template
    ///
    /// If the change between the old and new template results in a mapping that doesn't exist, then we need to bail out.
    /// Basically if we *had* a mapping of `[0, 1]` and the new template is `[1, 2]`, then we need to bail out, since
    /// the new mapping doesn't exist in the original.
    fn update_dynamic_node(&mut self, root: &'a BodyNode) -> Option<TemplateNode> {
        let idx = self.dynamic_nodes.len();

        // Put the node in the dynamic nodes list
        self.dynamic_nodes.push(root);

        // Fill in as many paths as we need - might have to fill in more since the old tempate shrunk some and let some paths be empty
        if self.node_paths.len() <= idx {
            self.node_paths.resize_with(idx + 1, Vec::new);
        }

        // And then set the path of this node to the current path (which we're hitting during traversal)
        self.node_paths[idx].clone_from(&self.current_path);

        Some(match root {
            BodyNode::Text(_) => TemplateNode::DynamicText { id: idx },
            _ => TemplateNode::Dynamic { id: idx },
        })
    }

    fn update_element<Ctx: HotReloadingContext>(
        &mut self,
        el: &'a Element,
    ) -> Option<TemplateNode> {
        let rust_name = el.name.to_string();

        let mut static_attr_array = Vec::new();

        for attr in &el.merged_attributes {
            let template_attr = match attr.as_static_str_literal() {
                // For static attributes, we don't need to pull in any mapping or anything
                // We can just build them directly
                Some((name, value)) => Self::make_static_attribute::<Ctx>(value, name, &rust_name),

                // For dynamic attributes, we need to check the mapping to see if that mapping exists
                // todo: one day we could generate new dynamic attributes on the fly if they're a literal,
                // or something sufficiently serializable
                //  (ie `checked`` being a bool and bools being interpretable)
                //
                // For now, just give up if that attribute doesn't exist in the mapping
                None => {
                    let id = self.update_dynamic_attribute(attr)?;
                    TemplateAttribute::Dynamic { id }
                }
            };

            static_attr_array.push(template_attr);
        }

        let children = self.fill::<Ctx>(el.children.as_slice())?;

        let (tag, namespace) =
            Ctx::map_element(&rust_name).unwrap_or((intern(rust_name.as_str()), None));

        Some(TemplateNode::Element {
            tag,
            namespace,
            attrs: intern(static_attr_array.into_boxed_slice()),
            children: intern(children.as_slice()),
        })
    }

    fn update_dynamic_attribute(&mut self, attr: &'a AttributeType) -> Option<usize> {
        let idx = self.dynamic_attributes.len();

        self.dynamic_attributes.push(vec![attr]);
        if self.attr_paths.len() <= idx {
            self.attr_paths.resize_with(idx + 1, Vec::new);
        }

        self.attr_paths[idx].clone_from(&self.current_path);

        Some(idx)
    }

    fn make_static_attribute<Ctx: HotReloadingContext>(
        value: &IfmtInput,
        name: &ElementAttrName,
        element_name_rust: &str,
    ) -> TemplateAttribute {
        let value = value.source.as_ref().unwrap();
        let attribute_name_rust = name.to_string();
        let (name, namespace) = Ctx::map_attribute(element_name_rust, &attribute_name_rust)
            .unwrap_or((intern(attribute_name_rust.as_str()), None));

        let static_attr = TemplateAttribute::Static {
            name,
            namespace,
            value: intern(value.value().as_str()),
        };

        static_attr
    }
}