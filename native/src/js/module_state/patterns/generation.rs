//! Generation numbers are opaque identities, not values that must restart at
//! zero on a page visit. Every read of the counter and its snapshots is checked.

use super::*;

impl Patterns<'_, '_> {
    pub(super) fn generation(&self, symbol: SymbolId) -> bool {
        if self.exported(symbol)
            || self.semantic.scoping().root_unresolved_references().contains_key("eval")
            || !self.definitions.get(&symbol).is_some_and(|e|
                matches!(e.get_inner_expression(), Expression::NumericLiteral(n) if n.value.is_finite() && n.value.fract() == 0.0)) {
            return false;
        }
        let nodes = self.semantic.nodes();
        let mut snapshots = HashSet::new();
        let mut advances = 0;
        for reference in self.semantic.scoping().get_resolved_references(symbol) {
            if !self.step() {
                return false;
            }
            let parent = nodes.parent_kind(reference.node_id());
            if reference.is_write() {
                let advances_only = match parent {
                    AstKind::UpdateExpression(u) => u.operator.as_str() == "++",
                    AstKind::AssignmentExpression(a) => {
                        a.operator.as_str() == "+="
                            && matches!(a.right.get_inner_expression(), Expression::NumericLiteral(n) if n.value == 1.0)
                    }
                    _ => false,
                };
                // Using the result of ++counter leaks its absolute value.
                let update_node = nodes.parent_id(reference.node_id());
                if !advances_only
                    || !matches!(
                        nodes.parent_kind(update_node),
                        AstKind::ExpressionStatement(_)
                    )
                {
                    return false;
                }
                advances += 1;
            } else if let AstKind::VariableDeclarator(d) = parent {
                if !d
                    .init
                    .as_ref()
                    .is_some_and(|e| self.expression_symbol(e) == Some(symbol))
                {
                    return false;
                }
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    return false;
                };
                let Some(snapshot) = id.symbol_id.get() else {
                    return false;
                };
                if !self
                    .semantic
                    .scoping()
                    .symbol_flags(snapshot)
                    .is_const_variable()
                    || self.owner(d.node_id.get()).is_none()
                {
                    return false;
                }
                snapshots.insert(snapshot);
            } else if !matches!(parent, AstKind::BinaryExpression(b) if matches!(b.operator.as_str(), "===" | "!=="))
            {
                return false;
            }
        }
        if advances == 0 || snapshots.is_empty() {
            return false;
        }
        let mut compared = false;
        for binding in snapshots.iter().copied().chain(std::iter::once(symbol)) {
            for reference in self.semantic.scoping().get_resolved_references(binding) {
                if !self.step() {
                    return false;
                }
                if reference.is_write() {
                    if binding != symbol {
                        return false;
                    }
                    continue;
                }
                let parent = nodes.parent_kind(reference.node_id());
                if binding == symbol && matches!(parent, AstKind::VariableDeclarator(_)) {
                    continue;
                }
                let AstKind::BinaryExpression(b) = parent else {
                    return false;
                };
                if !matches!(b.operator.as_str(), "===" | "!==") {
                    return false;
                }
                let left = self.expression_symbol(&b.left);
                let right = self.expression_symbol(&b.right);
                if !((left == Some(symbol) && right.is_some_and(|s| snapshots.contains(&s)))
                    || (right == Some(symbol) && left.is_some_and(|s| snapshots.contains(&s))))
                {
                    return false;
                }
                compared = true;
            }
        }
        compared
    }
}
