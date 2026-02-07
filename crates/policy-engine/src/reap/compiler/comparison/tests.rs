//! Tests for comparison compilation.

#[cfg(test)]
mod tests {
    use crate::evaluators::reaper_dsl::Condition as DslCondition;
    use crate::reap::ast::{
        ComparisonLeft, ComparisonRight, Entity, EntityAttr, Expr, Operator, Value,
    };
    use crate::reap::compiler::comparison::compile_comparison;

    #[test]
    fn test_compile_user_equals() {
        let result = compile_comparison(
            ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "role".to_string(),
                index: None,
            }),
            Operator::Equal,
            ComparisonRight::Value(Value::String("admin".to_string())),
        )
        .unwrap();

        // Should compile to AttributeCompare (consolidated type)
        assert!(matches!(result, DslCondition::AttributeCompare(_)));
    }

    #[test]
    fn test_compile_action_equals() {
        let result = compile_comparison(
            ComparisonLeft::Expr(Expr::Variable("action".to_string())),
            Operator::Equal,
            ComparisonRight::Value(Value::String("read".to_string())),
        )
        .unwrap();

        assert!(matches!(
            result,
            DslCondition::ActionEquals { value }
            if value == "read"
        ));
    }

    #[test]
    fn test_compile_variable_comparison() {
        let result = compile_comparison(
            ComparisonLeft::Expr(Expr::Variable("x".to_string())),
            Operator::Equal,
            ComparisonRight::Value(Value::String("test".to_string())),
        )
        .unwrap();

        assert!(matches!(
            result,
            DslCondition::VariableEqualsLiteral { variable, .. }
            if variable == "x"
        ));
    }

    #[test]
    fn test_compile_cross_entity_comparison() {
        let result = compile_comparison(
            ComparisonLeft::EntityAttr(EntityAttr {
                entity: Entity::User,
                attribute: "level".to_string(),
                index: None,
            }),
            Operator::GreaterThan,
            ComparisonRight::EntityAttr(EntityAttr {
                entity: Entity::Resource,
                attribute: "required_level".to_string(),
                index: None,
            }),
        )
        .unwrap();

        // Should compile to CrossEntityCompare (consolidated type)
        assert!(matches!(result, DslCondition::CrossEntityCompare(_)));
    }
}
