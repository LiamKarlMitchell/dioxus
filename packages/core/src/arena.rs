use crate::{
    innerlude::DirtyScope, nodes::RenderReturn, nodes::VNode, virtual_dom::VirtualDom,
    AttributeValue, DynamicNode, ScopeId,
};

/// An Element's unique identifier.
///
/// `ElementId` is a `usize` that is unique across the entire VirtualDOM - but not unique across time. If a component is
/// unmounted, then the `ElementId` will be reused for a new component.
#[cfg_attr(feature = "serialize", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ElementId(pub usize);

/// An Element that can be bubbled to's unique identifier.
///
/// `ElementId` is a `usize` that is unique across the entire VirtualDOM - but not unique across time. If a component is
/// unmounted, then the `ElementId` will be reused for a new component.
#[cfg_attr(feature = "serialize", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BubbleId(pub usize);

#[derive(Debug, Clone, Copy)]
pub struct ElementRef<'a> {
    // the pathway of the real element inside the template
    pub(crate) path: ElementPath,

    // The actual template
    pub(crate) template: &'a VNode<'a>,

    // The scope the element belongs to
    pub(crate) scope: ScopeId,
}

#[derive(Clone, Copy, Debug)]
pub struct ElementPath {
    pub(crate) path: &'static [u8],
}

impl VirtualDom {
    pub(crate) fn next_element(&mut self) -> ElementId {
        ElementId(self.elements.insert(None))
    }

    pub(crate) fn next_element_ref(&mut self, element_ref: ElementRef) -> BubbleId {
        BubbleId(
            self.element_refs
                .insert(unsafe { std::mem::transmute(element_ref) }),
        )
    }

    pub(crate) fn reclaim(&mut self, el: ElementId) {
        self.try_reclaim(el)
            .unwrap_or_else(|| panic!("cannot reclaim {:?}", el));
    }

    pub(crate) fn try_reclaim(&mut self, el: ElementId) -> Option<()> {
        if el.0 == 0 {
            panic!(
                "Cannot reclaim the root element - {:#?}",
                std::backtrace::Backtrace::force_capture()
            );
        }

        self.elements.try_remove(el.0).map(|_| ())
    }

    pub(crate) fn set_template(&mut self, el: ElementId, element_ref: ElementRef) {
        match self.elements[el.0] {
            Some(bubble_id) => {
                self.element_refs[bubble_id.0] = unsafe { std::mem::transmute(element_ref) };
            }
            None => {
                self.elements[el.0] = Some(self.next_element_ref(element_ref));
            }
        }
    }

    pub(crate) fn update_template(&mut self, el: ElementId, node: &VNode) {
        let bubble_id = self.elements[el.0].unwrap();
        let node: *const VNode = node as *const _;
        self.element_refs[bubble_id.0].template = unsafe { std::mem::transmute(node) };
    }

    // Drop a scope and all its children
    //
    // Note: This will not remove any ids from the arena
    pub(crate) fn drop_scope(&mut self, id: ScopeId, recursive: bool) {
        self.dirty_scopes.remove(&DirtyScope {
            height: self.scopes[id.0].height(),
            id,
        });

        self.ensure_drop_safety(id);

        if recursive {
            if let Some(root) = self.scopes[id.0].try_root_node() {
                if let RenderReturn::Ready(node) = unsafe { root.extend_lifetime_ref() } {
                    self.drop_scope_inner(node)
                }
            }
        }

        let scope = &mut self.scopes[id.0];

        // Drop all the hooks once the children are dropped
        // this means we'll drop hooks bottom-up
        scope.hooks.get_mut().clear();
        {
            let context = scope.context();

            // Drop all the futures once the hooks are dropped
            for task_id in context.spawned_tasks.borrow_mut().drain() {
                context.tasks.remove(task_id);
            }
        }

        self.scopes.remove(id.0);
    }

    fn drop_scope_inner(&mut self, node: &VNode) {
        node.dynamic_nodes.iter().for_each(|node| match node {
            DynamicNode::Component(c) => {
                if let Some(f) = c.scope.get() {
                    self.drop_scope(f, true);
                }
                c.props.take();
            }
            DynamicNode::Fragment(nodes) => {
                nodes.iter().for_each(|node| self.drop_scope_inner(node))
            }
            DynamicNode::Placeholder(_) => {}
            DynamicNode::Text(_) => {}
        });
    }

    /// Descend through the tree, removing any borrowed props and listeners
    pub(crate) fn ensure_drop_safety(&self, scope_id: ScopeId) {
        let scope = &self.scopes[scope_id.0];

        // make sure we drop all borrowed props manually to guarantee that their drop implementation is called before we
        // run the hooks (which hold an &mut Reference)
        // recursively call ensure_drop_safety on all children
        let mut props = scope.borrowed_props.borrow_mut();
        props.drain(..).for_each(|comp| {
            let comp = unsafe { &*comp };
            match comp.scope.get() {
                Some(child) if child != scope_id => self.ensure_drop_safety(child),
                _ => (),
            }
            if let Ok(mut props) = comp.props.try_borrow_mut() {
                *props = None;
            }
        });

        // Now that all the references are gone, we can safely drop our own references in our listeners.
        let mut listeners = scope.attributes_to_drop.borrow_mut();
        listeners.drain(..).for_each(|listener| {
            let listener = unsafe { &*listener };
            match &listener.value {
                AttributeValue::Listener(l) => {
                    _ = l.take();
                }
                AttributeValue::Any(a) => {
                    _ = a.take();
                }
                _ => (),
            }
        });
    }
}

impl ElementPath {
    pub(crate) fn is_decendant(&self, small: &&[u8]) -> bool {
        small.len() <= self.path.len() && *small == &self.path[..small.len()]
    }
}

impl PartialEq<&[u8]> for ElementPath {
    fn eq(&self, other: &&[u8]) -> bool {
        self.path.eq(*other)
    }
}
