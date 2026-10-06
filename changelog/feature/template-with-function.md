New `with(values..., |names...| body)` templating function which allows you to
assign expressions to name bindings in a lambda. Expressions are re-evaluated
for each use, so this is effectively the same as defining temporary template
aliases for each expression (currently no performance benefit).
