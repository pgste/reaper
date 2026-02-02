//! Tests for the Reaper DSL types.

#[cfg(test)]
mod tests {
    use crate::data::StringInterner;
    use crate::evaluators::reaper_dsl::types::*;

    #[test]
    fn test_numeric_op_conversion() {
        assert_eq!(AttrCompareOp::from(NumericOp::Equal), AttrCompareOp::Equal);
        assert_eq!(AttrCompareOp::from(NumericOp::Greater), AttrCompareOp::Greater);
        assert_eq!(NumericOp::from(AttrCompareOp::Less), NumericOp::Less);
    }

    #[test]
    fn test_attribute_comparison_creation() {
        let comp = AttributeComparison {
            entity_type: EntityType::User,
            attribute: "age".to_string(),
            op: NumericOp::GreaterEqual,
            target: CompareTarget::LiteralNum(18.0),
        };
        assert!(matches!(comp.entity_type, EntityType::User));
        assert_eq!(comp.attribute, "age");
    }

    #[test]
    fn test_string_operation_creation() {
        let op = StringOperationCondition {
            entity_type: EntityType::User,
            attribute: "email".to_string(),
            op: StringOp::Contains,
            value: "@company.com".to_string(),
        };
        assert!(matches!(op.op, StringOp::Contains));
    }

    #[test]
    fn test_count_condition_creation() {
        let cond = CountCondition {
            entity_type: EntityType::User,
            attribute: "roles".to_string(),
            op: CountOp::GreaterEqual,
            threshold: 1,
        };
        assert_eq!(cond.threshold, 1);
    }

    #[test]
    fn test_cross_entity_comparison() {
        let comp = CrossEntityComparison {
            left_entity: EntityType::User,
            left_attr: "level".to_string(),
            op: NumericOp::Greater,
            right_entity: EntityType::Resource,
            right_attr: "required_level".to_string(),
        };
        assert!(matches!(comp.left_entity, EntityType::User));
        assert!(matches!(comp.right_entity, EntityType::Resource));
    }

    #[test]
    fn test_wildcard_comparison() {
        let comp = WildcardComparison {
            collection_entity: EntityType::User,
            collection_attr: "roles".to_string(),
            scalar_entity: EntityType::Resource,
            scalar_attr: "required_role".to_string(),
        };
        assert!(matches!(comp.collection_entity, EntityType::User));
    }

    #[test]
    fn test_time_condition() {
        let cond = TimeCondition {
            entity_type: EntityType::User,
            attribute: "expires_at".to_string(),
            op: NumericOp::Greater,
            threshold: 1700000000,
        };
        assert_eq!(cond.threshold, 1700000000);
    }

    #[test]
    fn test_variable_string_operation() {
        let op = VariableStringOperationCondition {
            variable: "email".to_string(),
            op: StringOp::EndsWith,
            value: ".com".to_string(),
        };
        assert!(matches!(op.op, StringOp::EndsWith));
    }

    #[test]
    fn test_compare_target_variants() {
        let targets = vec![
            CompareTarget::LiteralString("admin".to_string()),
            CompareTarget::LiteralNum(42.0),
            CompareTarget::LiteralBool(true),
            CompareTarget::EntityAttr {
                entity_type: EntityType::Resource,
                attribute: "owner".to_string(),
            },
            CompareTarget::Variable("role_var".to_string()),
        ];
        assert_eq!(targets.len(), 5);
    }

    // ============================================================================
    // V2 Compiled Type Tests
    // ============================================================================

    #[test]
    fn test_attribute_comparison_to_compiled() {
        let interner = StringInterner::new();
        let comp = AttributeComparison {
            entity_type: EntityType::User,
            attribute: "age".to_string(),
            op: NumericOp::GreaterEqual,
            target: CompareTarget::LiteralNum(18.0),
        };

        let compiled = comp.to_compiled(&interner);
        assert!(matches!(compiled.entity_type, EntityType::User));
        assert!(matches!(compiled.op, NumericOp::GreaterEqual));
        assert!(matches!(compiled.target, CompiledCompareTarget::LiteralNum(n) if n == 18.0));
    }

    #[test]
    fn test_string_operation_to_compiled() {
        let interner = StringInterner::new();
        let op = StringOperationCondition {
            entity_type: EntityType::User,
            attribute: "email".to_string(),
            op: StringOp::Contains,
            value: "@company.com".to_string(),
        };

        let compiled = op.to_compiled(&interner);
        assert!(matches!(compiled.entity_type, EntityType::User));
        assert!(matches!(compiled.op, StringOp::Contains));
        assert_eq!(compiled.value, "@company.com");
    }

    #[test]
    fn test_count_condition_to_compiled() {
        let interner = StringInterner::new();
        let cond = CountCondition {
            entity_type: EntityType::User,
            attribute: "roles".to_string(),
            op: CountOp::GreaterEqual,
            threshold: 1,
        };

        let compiled = cond.to_compiled(&interner);
        assert!(matches!(compiled.entity_type, EntityType::User));
        assert!(matches!(compiled.op, CountOp::GreaterEqual));
        assert_eq!(compiled.threshold, 1);
    }

    #[test]
    fn test_time_condition_to_compiled() {
        let interner = StringInterner::new();
        let cond = TimeCondition {
            entity_type: EntityType::User,
            attribute: "expires_at".to_string(),
            op: NumericOp::Greater,
            threshold: 1700000000,
        };

        let compiled = cond.to_compiled(&interner);
        assert!(matches!(compiled.entity_type, EntityType::User));
        assert!(matches!(compiled.op, NumericOp::Greater));
        assert_eq!(compiled.threshold, 1700000000);
    }

    #[test]
    fn test_cross_entity_comparison_to_compiled() {
        let interner = StringInterner::new();
        let comp = CrossEntityComparison {
            left_entity: EntityType::User,
            left_attr: "level".to_string(),
            op: NumericOp::Greater,
            right_entity: EntityType::Resource,
            right_attr: "required_level".to_string(),
        };

        let compiled = comp.to_compiled(&interner);
        assert!(matches!(compiled.left_entity, EntityType::User));
        assert!(matches!(compiled.right_entity, EntityType::Resource));
        assert!(matches!(compiled.op, NumericOp::Greater));
    }

    #[test]
    fn test_wildcard_comparison_to_compiled() {
        let interner = StringInterner::new();
        let comp = WildcardComparison {
            collection_entity: EntityType::User,
            collection_attr: "roles".to_string(),
            scalar_entity: EntityType::Resource,
            scalar_attr: "required_role".to_string(),
        };

        let compiled = comp.to_compiled(&interner);
        assert!(matches!(compiled.collection_entity, EntityType::User));
        assert!(matches!(compiled.scalar_entity, EntityType::Resource));
    }

    #[test]
    fn test_variable_string_operation_to_compiled() {
        let interner = StringInterner::new();
        let op = VariableStringOperationCondition {
            variable: "email".to_string(),
            op: StringOp::EndsWith,
            value: ".com".to_string(),
        };

        let compiled = op.to_compiled(&interner);
        assert!(matches!(compiled.op, StringOp::EndsWith));
        assert_eq!(compiled.value, ".com");
    }

    // ============================================================================
    // CompiledCondition V2 Extraction Tests
    // ============================================================================

    #[test]
    fn test_extract_attribute_comparison_from_compiled_condition() {
        let interner = StringInterner::new();
        let attr = interner.intern("level");

        let compiled = CompiledCondition::AttributeCompare(CompiledAttributeComparison {
            entity_type: EntityType::User,
            attribute: attr,
            op: NumericOp::GreaterEqual,
            target: CompiledCompareTarget::LiteralNum(5.0),
        });

        let extracted = compiled.as_attribute_comparison();
        assert!(extracted.is_some());

        let comp = extracted.unwrap();
        assert!(matches!(comp.entity_type, EntityType::User));
        assert!(matches!(comp.op, NumericOp::GreaterEqual));
    }

    #[test]
    fn test_extract_string_operation_from_compiled_condition() {
        let interner = StringInterner::new();
        let attr = interner.intern("email");

        let compiled = CompiledCondition::StringOp(CompiledStringOperation {
            entity_type: EntityType::User,
            attribute: attr,
            op: StringOp::Contains,
            value: "@test.com".to_string(),
        });

        let extracted = compiled.as_string_operation();
        assert!(extracted.is_some());

        let op = extracted.unwrap();
        assert!(matches!(op.entity_type, EntityType::User));
        assert!(matches!(op.op, StringOp::Contains));
        assert_eq!(op.value, "@test.com");
    }

    #[test]
    fn test_extract_count_condition_from_compiled_condition() {
        let interner = StringInterner::new();
        let attr = interner.intern("items");

        let compiled = CompiledCondition::CountOp(CompiledCountCondition {
            entity_type: EntityType::Resource,
            attribute: attr,
            op: CountOp::Greater,
            threshold: 10,
        });

        let extracted = compiled.as_count_condition();
        assert!(extracted.is_some());

        let cond = extracted.unwrap();
        assert!(matches!(cond.entity_type, EntityType::Resource));
        assert!(matches!(cond.op, CountOp::Greater));
        assert_eq!(cond.threshold, 10);
    }

    #[test]
    fn test_extract_time_condition_from_compiled_condition() {
        let interner = StringInterner::new();
        let attr = interner.intern("created_at");

        let compiled = CompiledCondition::TimeOp(CompiledTimeCondition {
            entity_type: EntityType::User,
            attribute: attr,
            op: NumericOp::Less,
            threshold: 1600000000,
        });

        let extracted = compiled.as_time_condition();
        assert!(extracted.is_some());

        let cond = extracted.unwrap();
        assert!(matches!(cond.entity_type, EntityType::User));
        assert!(matches!(cond.op, NumericOp::Less));
        assert_eq!(cond.threshold, 1600000000);
    }

    #[test]
    fn test_count_op_to_attr_compare_op() {
        assert_eq!(AttrCompareOp::from(CountOp::GreaterEqual), AttrCompareOp::GreaterEqual);
        assert_eq!(AttrCompareOp::from(CountOp::Greater), AttrCompareOp::Greater);
        assert_eq!(AttrCompareOp::from(CountOp::Equal), AttrCompareOp::Equal);
        assert_eq!(AttrCompareOp::from(CountOp::Less), AttrCompareOp::Less);
        assert_eq!(AttrCompareOp::from(CountOp::LessEqual), AttrCompareOp::LessEqual);
    }

    // ============================================================================
    // V2 Condition Variant Tests
    // ============================================================================

    #[test]
    fn test_condition_v2_attribute_compare() {
        let cond = Condition::AttributeCompare(AttributeComparison {
            entity_type: EntityType::User,
            attribute: "age".to_string(),
            op: NumericOp::GreaterEqual,
            target: CompareTarget::LiteralNum(18.0),
        });

        if let Condition::AttributeCompare(comp) = cond {
            assert!(matches!(comp.entity_type, EntityType::User));
            assert_eq!(comp.attribute, "age");
            assert!(matches!(comp.op, NumericOp::GreaterEqual));
        } else {
            panic!("Expected AttributeCompare");
        }
    }

    #[test]
    fn test_condition_v2_string_op() {
        let cond = Condition::StringOp(StringOperationCondition {
            entity_type: EntityType::User,
            attribute: "email".to_string(),
            op: StringOp::Contains,
            value: "@company.com".to_string(),
        });

        if let Condition::StringOp(op) = cond {
            assert!(matches!(op.entity_type, EntityType::User));
            assert_eq!(op.attribute, "email");
            assert!(matches!(op.op, StringOp::Contains));
            assert_eq!(op.value, "@company.com");
        } else {
            panic!("Expected StringOp");
        }
    }

    #[test]
    fn test_condition_v2_count_op() {
        let cond = Condition::CountOp(CountCondition {
            entity_type: EntityType::Resource,
            attribute: "items".to_string(),
            op: CountOp::Greater,
            threshold: 10,
        });

        if let Condition::CountOp(c) = cond {
            assert!(matches!(c.entity_type, EntityType::Resource));
            assert_eq!(c.attribute, "items");
            assert!(matches!(c.op, CountOp::Greater));
            assert_eq!(c.threshold, 10);
        } else {
            panic!("Expected CountOp");
        }
    }

    #[test]
    fn test_condition_v2_cross_entity_compare() {
        let cond = Condition::CrossEntityCompare(CrossEntityComparison {
            left_entity: EntityType::User,
            left_attr: "level".to_string(),
            op: NumericOp::Greater,
            right_entity: EntityType::Resource,
            right_attr: "required_level".to_string(),
        });

        if let Condition::CrossEntityCompare(comp) = cond {
            assert!(matches!(comp.left_entity, EntityType::User));
            assert!(matches!(comp.right_entity, EntityType::Resource));
            assert!(matches!(comp.op, NumericOp::Greater));
        } else {
            panic!("Expected CrossEntityCompare");
        }
    }

    #[test]
    fn test_condition_v2_wildcard_compare() {
        let cond = Condition::WildcardCompare(WildcardComparison {
            collection_entity: EntityType::User,
            collection_attr: "roles".to_string(),
            scalar_entity: EntityType::Resource,
            scalar_attr: "required_role".to_string(),
        });

        if let Condition::WildcardCompare(comp) = cond {
            assert!(matches!(comp.collection_entity, EntityType::User));
            assert!(matches!(comp.scalar_entity, EntityType::Resource));
        } else {
            panic!("Expected WildcardCompare");
        }
    }
}
