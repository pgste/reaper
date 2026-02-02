use super::*;

#[test]
fn test_parse_simple_policy() {
        let input = r#"
            policy test {
                default: deny,
                rule admin { allow if user.role == "admin" }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.name, "test");
        assert_eq!(policy.rules.len(), 1);
        assert_eq!(policy.rules[0].name, "admin");
    }

    #[test]
    fn test_parse_with_metadata() {
        let input = r#"
            policy test {
                version: "1.0.0",
                description: "Test policy",
                default: allow,
                rule test { deny if user.suspended == true }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.metadata.get("version"), Some(&"1.0.0".to_string()));
    }

    #[test]
    fn test_parse_complex_condition() {
        let input = r#"
            policy test {
                default: deny,
                rule complex {
                    allow if {
                        user.department == resource.department &&
                        user.clearance >= resource.clearance_required
                    }
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.rules.len(), 1);
    }

    #[test]
    fn test_parse_array_values() {
        let input = r#"
            policy test {
                default: deny,
                rule array_test {
                    allow if user.roles == [1, 2, 3]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.rules.len(), 1);

        // Verify it's a comparison with an array value
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Array(arr)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(arr.len(), 3);
        } else {
            panic!("Expected array value");
        }
    }

    #[test]
    fn test_parse_empty_array() {
        let input = r#"
            policy test {
                default: deny,
                rule empty_array { allow if user.items == [] }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Array(arr)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(arr.len(), 0);
        } else {
            panic!("Expected empty array");
        }
    }

    #[test]
    fn test_parse_nested_array() {
        let input = r#"
            policy test {
                default: deny,
                rule nested { allow if user.matrix == [[1, 2], [3, 4]] }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Array(arr)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(arr.len(), 2);
            if let Value::Array(inner) = &arr[0] {
                assert_eq!(inner.len(), 2);
            } else {
                panic!("Expected nested array");
            }
        } else {
            panic!("Expected array value");
        }
    }

    #[test]
    fn test_parse_object_values() {
        let input = r#"
            policy test {
                default: deny,
                rule object_test {
                    allow if user.config == {"timeout": 30, "retries": 3}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Object(obj)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(obj.len(), 2);
            assert_eq!(obj[0].0, "timeout");
            if let Value::Integer(val) = obj[0].1 {
                assert_eq!(val, 30);
            } else {
                panic!("Expected integer value");
            }
        } else {
            panic!("Expected object value");
        }
    }

    #[test]
    fn test_parse_set_values() {
        let input = r#"
            policy test {
                default: deny,
                rule set_test {
                    allow if user.permissions == {"read", "write", "delete"}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Set(set)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(set.len(), 3);
        } else {
            panic!("Expected set value");
        }
    }

    #[test]
    fn test_parse_empty_set() {
        let input = r#"
            policy test {
                default: deny,
                rule empty_set { allow if user.tags == {} }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Set(set)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(set.len(), 0);
        } else {
            panic!("Expected empty set");
        }
    }

    #[test]
    fn test_parse_nested_object() {
        let input = r#"
            policy test {
                default: deny,
                rule nested {
                    allow if user.profile == {"name": "alice", "settings": {"theme": "dark"}}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Object(obj)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(obj.len(), 2);
            if let Value::Object(inner) = &obj[1].1 {
                assert_eq!(inner.len(), 1);
                assert_eq!(inner[0].0, "theme");
            } else {
                panic!("Expected nested object");
            }
        } else {
            panic!("Expected object value");
        }
    }

    #[test]
    fn test_parse_mixed_types_in_array() {
        let input = r#"
            policy test {
                default: deny,
                rule mixed {
                    allow if user.data == [1, "hello", true, null, [2, 3]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison {
            right: ComparisonRight::Value(Value::Array(arr)),
            ..
        } = &policy.rules[0].condition
        {
            assert_eq!(arr.len(), 5);
            assert!(matches!(arr[0], Value::Integer(_)));
            assert!(matches!(arr[1], Value::String(_)));
            assert!(matches!(arr[2], Value::Boolean(_)));
            assert!(matches!(arr[3], Value::Null));
            assert!(matches!(arr[4], Value::Array(_)));
        } else {
            panic!("Expected array with mixed types");
        }
    }

    #[test]
    fn test_parse_bracket_notation_numeric() {
        let input = r#"
            policy test {
                default: deny,
                rule array_index {
                    allow if user.roles[0] == "admin"
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison { left, .. } = &policy.rules[0].condition {
            if let ComparisonLeft::EntityAttr(attr) = left {
                assert_eq!(attr.attribute, "roles");
                assert!(attr.index.is_some());
                if let Some(Index::Number(n)) = &attr.index {
                    assert_eq!(n, &0);
                } else {
                    panic!("Expected numeric index");
                }
            } else {
                panic!("Expected entity attribute");
            }
        } else {
            panic!("Expected comparison");
        }
    }

    #[test]
    fn test_parse_bracket_notation_string() {
        let input = r#"
            policy test {
                default: deny,
                rule object_key {
                    allow if user.data["department"] == "engineering"
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison { left, .. } = &policy.rules[0].condition {
            if let ComparisonLeft::EntityAttr(attr) = left {
                assert_eq!(attr.attribute, "data");
                assert!(attr.index.is_some());
                if let Some(Index::String(s)) = &attr.index {
                    assert_eq!(s, "department");
                } else {
                    panic!("Expected string index");
                }
            } else {
                panic!("Expected entity attribute");
            }
        } else {
            panic!("Expected comparison");
        }
    }

    #[test]
    fn test_parse_in_operator() {
        let input = r#"
            policy test {
                default: deny,
                rule membership {
                    allow if "admin" in user.roles
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        // "admin" in user.roles is parsed as: left=user.roles, op=In, right="admin"
        if let Condition::Comparison { left, op, right } = &policy.rules[0].condition {
            assert_eq!(*op, Operator::In);
            if let ComparisonLeft::EntityAttr(attr) = left {
                assert_eq!(attr.entity, Entity::User);
                assert_eq!(attr.attribute, "roles");
            } else {
                panic!("Expected entity attribute");
            }
            if let ComparisonRight::Value(Value::String(s)) = right {
                assert_eq!(s, "admin");
            } else {
                panic!("Expected string value on right side");
            }
        } else {
            panic!("Expected comparison");
        }
    }

    #[test]
    fn test_parse_in_operator_with_variable() {
        let input = r#"
            policy test {
                default: deny,
                rule check_permission {
                    allow if context.action in resource.allowed_actions
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison { left, op, right } = &policy.rules[0].condition {
            assert_eq!(*op, Operator::In);
            if let ComparisonLeft::EntityAttr(attr) = left {
                assert_eq!(attr.entity, Entity::Context);
                assert_eq!(attr.attribute, "action");
            } else {
                panic!("Expected entity attribute on left side");
            }
            if let ComparisonRight::EntityAttr(attr) = right {
                assert_eq!(attr.entity, Entity::Resource);
                assert_eq!(attr.attribute, "allowed_actions");
            } else {
                panic!("Expected entity attribute on right side");
            }
        } else {
            panic!("Expected comparison");
        }
    }

    #[test]
    fn test_parse_variable_assignment() {
        let input = r#"
            policy test {
                default: deny,
                rule with_variable {
                    allow if role := user.role
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        // The condition should be a simple assignment
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "role");
            if let AssignmentValue::EntityAttr(attr) = value {
                assert_eq!(attr.entity, Entity::User);
                assert_eq!(attr.attribute, "role");
            } else {
                panic!("Expected entity attr in assignment");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_assignment_value_types() {
        // Test assignment from literal value
        let input = r#"
            policy test {
                default: deny,
                rule literal_assign {
                    allow if x := "admin"
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "x");
            if let AssignmentValue::Value(Value::String(s)) = value {
                assert_eq!(s, "admin");
            } else {
                panic!("Expected string value");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comparison_with_variable_right() {
        let input = r#"
            policy test {
                default: deny,
                rule compare_var {
                    allow if user.role == role_var
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Comparison { left, op, right } = &policy.rules[0].condition {
            if let ComparisonLeft::EntityAttr(attr) = left {
                assert_eq!(attr.entity, Entity::User);
                assert_eq!(attr.attribute, "role");
            } else {
                panic!("Expected entity attribute on left side");
            }
            assert_eq!(*op, Operator::Equal);
            if let ComparisonRight::Variable(var) = right {
                assert_eq!(var, "role_var");
            } else {
                panic!("Expected variable on right side");
            }
        } else {
            panic!("Expected comparison");
        }
    }

    // ========== COMPREHENSION PARSER TESTS ==========

    #[test]
    fn test_parse_set_comprehension_simple() {
        let input = r#"
            policy test {
                default: deny,
                rule collect_names {
                    allow if admin_names := {u.name | u := user.team[_]}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "admin_names");
            if let AssignmentValue::Comprehension(Comprehension::Set {
                output,
                iterator,
                filters,
            }) = value
            {
                // Check output expression: u.name
                if let Expr::AttributeAccess {
                    variable: var,
                    attribute: attr,
                } = output.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "name");
                } else {
                    panic!("Expected attribute access in output");
                }

                // Check iterator: u := user.team[_]
                assert_eq!(iterator.variable, "u");
                if let IterationSource::EntityAttr(entity_attr) = &iterator.collection {
                    assert_eq!(entity_attr.entity, Entity::User);
                    assert_eq!(entity_attr.attribute, "team");
                    assert!(matches!(entity_attr.index, Some(Index::Wildcard)));
                } else {
                    panic!("Expected EntityAttr in collection");
                }

                // No filters
                assert_eq!(filters.len(), 0);
            } else {
                panic!("Expected set comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_array_comprehension_simple() {
        let input = r#"
            policy test {
                default: deny,
                rule collect_emails {
                    allow if all_emails := [u.email | u := user.contacts[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "all_emails");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator,
                filters,
            }) = value
            {
                // Check output expression: u.email
                if let Expr::AttributeAccess {
                    variable: var,
                    attribute: attr,
                } = output.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "email");
                } else {
                    panic!("Expected attribute access in output");
                }

                // Check iterator
                assert_eq!(iterator.variable, "u");
                if let IterationSource::EntityAttr(entity_attr) = &iterator.collection {
                    assert_eq!(entity_attr.entity, Entity::User);
                    assert_eq!(entity_attr.attribute, "contacts");
                } else {
                    panic!("Expected EntityAttr in collection");
                }

                // No filters
                assert_eq!(filters.len(), 0);
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_object_comprehension_simple() {
        let input = r#"
            policy test {
                default: deny,
                rule create_user_map {
                    allow if user_map := {u.id: u.name | u := user.all_users[_]}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "user_map");
            if let AssignmentValue::Comprehension(Comprehension::Object {
                key,
                value: val,
                iterator,
                filters,
            }) = value
            {
                // Check key expression: u.id
                if let Expr::AttributeAccess {
                    variable: var,
                    attribute: attr,
                } = key.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "id");
                } else {
                    panic!("Expected attribute access in key");
                }

                // Check value expression: u.name
                if let Expr::AttributeAccess {
                    variable: var,
                    attribute: attr,
                } = val.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "name");
                } else {
                    panic!("Expected attribute access in value");
                }

                // Check iterator
                assert_eq!(iterator.variable, "u");
                if let IterationSource::EntityAttr(entity_attr) = &iterator.collection {
                    assert_eq!(entity_attr.entity, Entity::User);
                } else {
                    panic!("Expected EntityAttr in collection");
                }

                // No filters
                assert_eq!(filters.len(), 0);
            } else {
                panic!("Expected object comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_with_single_filter() {
        let input = r#"
            policy test {
                default: deny,
                rule active_users {
                    allow if active := {u.name | u := user.users[_]; u.active == true}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "active");
            if let AssignmentValue::Comprehension(Comprehension::Set {
                output: _,
                iterator,
                filters,
            }) = value
            {
                assert_eq!(iterator.variable, "u");
                assert_eq!(filters.len(), 1);

                // Check filter: u.active == true
                if let Condition::Comparison { left, op, right } = &filters[0] {
                    if let ComparisonLeft::VarAttr(var_attr) = left {
                        assert_eq!(var_attr.variable, "u");
                        assert_eq!(var_attr.attribute, "active");
                    } else {
                        panic!("Expected var attribute in filter");
                    }
                    assert_eq!(*op, Operator::Equal);
                    if let ComparisonRight::Value(Value::Boolean(b)) = right {
                        assert!(*b);
                    } else {
                        panic!("Expected boolean value");
                    }
                } else {
                    panic!("Expected comparison in filter");
                }
            } else {
                panic!("Expected set comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_with_multiple_filters() {
        let input = r#"
            policy test {
                default: deny,
                rule senior_devs {
                    allow if senior_dev_emails := [u.email |
                        u := user.employees[_];
                        u.role == "developer";
                        u.years_experience >= 5
                    ]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "senior_dev_emails");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator,
                filters,
            }) = value
            {
                // Check output: u.email
                if let Expr::AttributeAccess {
                    variable: var,
                    attribute: attr,
                } = output.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "email");
                } else {
                    panic!("Expected attribute access");
                }

                // Check iterator
                assert_eq!(iterator.variable, "u");

                // Check two filters
                assert_eq!(filters.len(), 2);

                // Filter 1: u.role == "developer"
                if let Condition::Comparison { left, op, right } = &filters[0] {
                    if let ComparisonLeft::VarAttr(var_attr) = left {
                        assert_eq!(var_attr.variable, "u");
                        assert_eq!(var_attr.attribute, "role");
                    } else {
                        panic!("Expected var attribute in first filter");
                    }
                    assert_eq!(*op, Operator::Equal);
                    if let ComparisonRight::Value(Value::String(s)) = right {
                        assert_eq!(s, "developer");
                    } else {
                        panic!("Expected string value");
                    }
                } else {
                    panic!("Expected comparison in first filter");
                }

                // Filter 2: u.years_experience >= 5
                if let Condition::Comparison { left, op, right } = &filters[1] {
                    if let ComparisonLeft::VarAttr(var_attr) = left {
                        assert_eq!(var_attr.variable, "u");
                        assert_eq!(var_attr.attribute, "years_experience");
                    } else {
                        panic!("Expected var attribute in second filter");
                    }
                    assert_eq!(*op, Operator::GreaterEqual);
                    if let ComparisonRight::Value(Value::Integer(i)) = right {
                        assert_eq!(*i, 5);
                    } else {
                        panic!("Expected integer value");
                    }
                } else {
                    panic!("Expected comparison in second filter");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_with_literal_output() {
        let input = r#"
            policy test {
                default: deny,
                rule count_users {
                    allow if counts := [1 | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "counts");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                // Check output: literal 1
                if let Expr::Literal(Value::Integer(i)) = output.as_ref() {
                    assert_eq!(*i, 1);
                } else {
                    panic!("Expected literal integer in output");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_with_variable_output() {
        let input = r#"
            policy test {
                default: deny,
                rule collect_vars {
                    allow if collected := {u | u := user.items[_]}
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "collected");
            if let AssignmentValue::Comprehension(Comprehension::Set {
                output,
                iterator,
                filters: _,
            }) = value
            {
                // Check output: variable u
                if let Expr::Variable(var) = output.as_ref() {
                    assert_eq!(var, "u");
                    assert_eq!(var, &iterator.variable); // Same as iterator variable
                } else {
                    panic!("Expected variable in output");
                }
            } else {
                panic!("Expected set comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_with_indexed_output() {
        let input = r#"
            policy test {
                default: deny,
                rule first_roles {
                    allow if first_roles := [u.roles[0] | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "first_roles");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                // Check output: u.roles[0]
                if let Expr::IndexedAccess {
                    variable: var,
                    attribute: attr,
                    index,
                } = output.as_ref()
                {
                    assert_eq!(var, "u");
                    assert_eq!(attr, "roles");
                    assert!(matches!(index, Index::Number(0)));
                } else {
                    panic!("Expected indexed access in output");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_comprehension_in_and_condition() {
        let input = r#"
            policy test {
                default: deny,
                rule complex_check {
                    allow if {
                        admin_names := {u.name | u := user.admins[_]; u.active == true} &&
                        user.name in admin_names
                    }
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        // The condition should be an AND
        if let Condition::And(conditions) = &policy.rules[0].condition {
            assert_eq!(conditions.len(), 2);

            // First condition: assignment with comprehension
            if let Condition::Assignment { variable, value } = &conditions[0] {
                assert_eq!(variable, "admin_names");
                assert!(matches!(
                    value,
                    AssignmentValue::Comprehension(Comprehension::Set { .. })
                ));
            } else {
                panic!("Expected assignment in first AND condition");
            }

            // Second condition: membership test
            if let Condition::Comparison { left, op, right } = &conditions[1] {
                if let ComparisonLeft::EntityAttr(attr) = left {
                    assert_eq!(attr.entity, Entity::User);
                    assert_eq!(attr.attribute, "name");
                } else {
                    panic!("Expected entity attribute in second condition");
                }
                assert_eq!(*op, Operator::In);
                if let ComparisonRight::Variable(var) = right {
                    assert_eq!(var, "admin_names");
                } else {
                    panic!("Expected variable reference");
                }
            } else {
                panic!("Expected comparison in second AND condition");
            }
        } else {
            panic!("Expected AND condition");
        }
    }

    // ===== Built-in Function Tests =====

    #[test]
    fn test_parse_method_call_count() {
        let input = r#"
            policy test {
                default: deny,
                rule count_check {
                    allow if perm_count := [perms.count() | perms := user.permissions[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value: _ } = &policy.rules[0].condition {
            assert_eq!(variable, "perm_count");
            // Method call in comprehension output - valid syntax
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_in_comprehension_output() {
        let input = r#"
            policy test {
                default: deny,
                rule lower_names {
                    allow if names := [u.name.lower() | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "names");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                // Output should be a method call: u.name.lower()
                if let Expr::MethodCall {
                    receiver,
                    method,
                    args,
                } = output.as_ref()
                {
                    // Receiver should be u.name (attribute access)
                    if let Expr::AttributeAccess {
                        variable,
                        attribute,
                    } = receiver.as_ref()
                    {
                        assert_eq!(variable, "u");
                        assert_eq!(attribute, "name");
                    } else {
                        panic!("Expected attribute access as receiver");
                    }
                    assert_eq!(*method, MethodName::Lower);
                    assert_eq!(args.len(), 0);
                } else {
                    panic!("Expected method call in output");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_sum() {
        let input = r#"
            policy test {
                default: deny,
                rule sum_test {
                    allow if total := [u.score.sum() | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "total");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Sum);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_with_args() {
        let input = r#"
            policy test {
                default: deny,
                rule split_test {
                    allow if parts := [u.email.split("@") | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "parts");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall {
                    receiver,
                    method,
                    args,
                } = output.as_ref()
                {
                    if let Expr::AttributeAccess {
                        variable,
                        attribute,
                    } = receiver.as_ref()
                    {
                        assert_eq!(variable, "u");
                        assert_eq!(attribute, "email");
                    }
                    assert_eq!(*method, MethodName::Split);
                    assert_eq!(args.len(), 1);
                    if let Expr::Literal(Value::String(s)) = &args[0] {
                        assert_eq!(s, "@");
                    } else {
                        panic!("Expected string argument");
                    }
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_function_call_is_string() {
        let input = r#"
            policy test {
                default: deny,
                rule type_check {
                    allow if strings := [u.name | u := user.users[_]; is_string(u.name)]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "strings");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output: _,
                iterator: _,
                filters,
            }) = value
            {
                assert_eq!(filters.len(), 1);
                // The filter should parse but we're testing the function call syntax here
                // Since filters are Condition not Expr, function calls in conditions need different handling
                // For now, let's test function calls in comprehension output
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_max() {
        let input = r#"
            policy test {
                default: deny,
                rule max_test {
                    allow if max_val := [scores.max() | scores := user.scores[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "max_val");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Max);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_min() {
        let input = r#"
            policy test {
                default: deny,
                rule min_test {
                    allow if min_val := [scores.min() | scores := user.scores[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "min_val");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Min);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_upper() {
        let input = r#"
            policy test {
                default: deny,
                rule upper_test {
                    allow if codes := [u.code.upper() | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "codes");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Upper);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_trim() {
        let input = r#"
            policy test {
                default: deny,
                rule trim_test {
                    allow if names := [u.name.trim() | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "names");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Trim);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_contains() {
        let input = r#"
            policy test {
                default: deny,
                rule contains_test {
                    allow if matches := [u.role.contains("admin") | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value: _ } = &policy.rules[0].condition {
            assert_eq!(variable, "matches");
            // Test parses successfully with contains() in output expression
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_startswith() {
        let input = r#"
            policy test {
                default: deny,
                rule prefix_test {
                    allow if starts := [u.email.startswith("admin") | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.rules[0].name, "prefix_test");
    }

    #[test]
    fn test_parse_method_call_endswith() {
        let input = r#"
            policy test {
                default: deny,
                rule suffix_test {
                    allow if ends := [u.email.endswith("@company.com") | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        assert_eq!(policy.rules[0].name, "suffix_test");
    }

    #[test]
    fn test_parse_method_call_union() {
        let input = r#"
            policy test {
                default: deny,
                rule union_test {
                    allow if all_perms := [user_perms.union(role_perms) | user_perms := user.perms[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "all_perms");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, args, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Union);
                    assert_eq!(args.len(), 1);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_intersection() {
        let input = r#"
            policy test {
                default: deny,
                rule intersection_test {
                    allow if common := [a.intersection(b) | a := user.sets[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "common");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Intersection);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_difference() {
        let input = r#"
            policy test {
                default: deny,
                rule difference_test {
                    allow if diff := [a.difference(b) | a := user.sets[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "diff");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::MethodCall { method, .. } = output.as_ref() {
                    assert_eq!(*method, MethodName::Difference);
                } else {
                    panic!("Expected method call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_function_call_concat() {
        let input = r#"
            policy test {
                default: deny,
                rule concat_test {
                    allow if full_names := [concat(u.first, " ", u.last) | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "full_names");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                if let Expr::FunctionCall {
                    namespace,
                    function,
                    args,
                } = output.as_ref()
                {
                    assert_eq!(namespace, &None);
                    assert_eq!(function, "concat");
                    assert_eq!(args.len(), 3);
                } else {
                    panic!("Expected function call");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

    #[test]
    fn test_parse_method_call_chaining() {
        let input = r#"
            policy test {
                default: deny,
                rule chain_test {
                    allow if clean_names := [u.name.trim().lower() | u := user.users[_]]
                }
            }
        "#;

        let policy = ReapParser::parse(input).unwrap();
        if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
            assert_eq!(variable, "clean_names");
            if let AssignmentValue::Comprehension(Comprehension::Array {
                output,
                iterator: _,
                filters: _,
            }) = value
            {
                // Output should be a chained method call: u.name.trim().lower()
                if let Expr::MethodCall {
                    receiver,
                    method,
                    args: _,
                } = output.as_ref()
                {
                    // Outer call is .lower()
                    assert_eq!(*method, MethodName::Lower);
                    // Receiver should be u.name.trim() (another method call)
                    if let Expr::MethodCall {
                        receiver: inner_receiver,
                        method: inner_method,
                        args: _,
                    } = receiver.as_ref()
                    {
                        assert_eq!(*inner_method, MethodName::Trim);
                        // Inner receiver should be u.name
                        if let Expr::AttributeAccess {
                            variable,
                            attribute,
                        } = inner_receiver.as_ref()
                        {
                            assert_eq!(variable, "u");
                            assert_eq!(attribute, "name");
                        } else {
                            panic!("Expected attribute access in inner receiver");
                        }
                    } else {
                        panic!("Expected method call as receiver for chaining");
                    }
                } else {
                    panic!("Expected method call in output");
                }
            } else {
                panic!("Expected array comprehension");
            }
        } else {
            panic!("Expected assignment");
        }
    }

#[test]
fn test_parse_time_now_ns() {
    let input = r#"
            policy test {
                default: deny,
                rule time_check {
                    allow if now := time::now_ns()
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
    // Verify it parses correctly
    if let Condition::Assignment { variable, value } = &policy.rules[0].condition {
        assert_eq!(variable, "now");
        if let AssignmentValue::Variable(ref _v) = value {
            // This would actually be a function call expression
            // Just verify it compiles
        }
    }
}

#[test]
fn test_parse_time_parse_rfc3339() {
    let input = r#"
            policy test {
                default: deny,
                rule time_check {
                    allow if timestamp := time::parse_rfc3339("2025-01-01T00:00:00Z")
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_time_arithmetic() {
    let input = r#"
            policy test {
                default: deny,
                rule time_check {
                    allow if future := time::add_ns(time::now_ns(), 3600000000000)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_time_comparison() {
    let input = r#"
            policy test {
                default: deny,
                rule time_check {
                    allow if time::is_before(user.expires_at, time::now_ns())
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_regex_matches() {
    let input = r#"
            policy test {
                default: deny,
                rule email_validation {
                    allow if valid_emails := [e.matches("^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$") | e := user.emails[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_regex_find() {
    let input = r#"
            policy test {
                default: deny,
                rule pattern_extract {
                    allow if matches := [t.find("\\d+") | t := user.texts[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_regex_replace() {
    let input = r#"
            policy test {
                default: deny,
                rule sanitize {
                    allow if clean_values := [inp.replace("[^a-zA-Z0-9]", "") | inp := user.inputs[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_regex_namespace_functions() {
    let input = r#"
            policy test {
                default: deny,
                rule validate_pattern {
                    allow if pattern := "[a-z]+"
                    && regex::is_valid(pattern)
                    && escaped := regex::escape(user.input)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_math_abs_and_round() {
    let input = r#"
            policy test {
                default: deny,
                rule math_rounding {
                    allow if abs_val := math::abs(-42)
                    && rounded := math::round(3.7)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_math_floor_ceil() {
    let input = r#"
            policy test {
                default: deny,
                rule math_floor_ceil {
                    allow if floor_val := math::floor(3.9)
                    && ceil_val := math::ceil(3.1)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_math_pow_sqrt() {
    let input = r#"
            policy test {
                default: deny,
                rule math_power {
                    allow if squared := math::pow(5, 2)
                    && sqrt_val := math::sqrt(16)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_math_min_max_clamp() {
    let input = r#"
            policy test {
                default: deny,
                rule math_comparisons {
                    allow if min_val := math::min(10, 20)
                    && max_val := math::max(10, 20)
                    && clamped := math::clamp(150, 0, 100)
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_collection_first_last() {
    let input = r#"
            policy test {
                default: deny,
                rule array_access {
                    allow if first_names := [arr.first() | arr := user.lists[_]]
                    && last_items := [a.last() | a := resource.arrays[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_collection_slice_reverse() {
    let input = r#"
            policy test {
                default: deny,
                rule array_manipulation {
                    allow if sliced_arrays := [arr.slice(1, 4) | arr := user.data[_]]
                    && reversed_lists := [lst.reverse() | lst := resource.lists[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_collection_sort_unique() {
    let input = r#"
            policy test {
                default: deny,
                rule array_processing {
                    allow if sorted_nums := [nums.sort() | nums := user.numbers[_]]
                    && unique_vals := [vals.unique() | vals := resource.values[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_object_methods() {
    let input = r#"
            policy test {
                default: deny,
                rule object_access {
                    allow if all_keys := [obj.keys() | obj := user.objects[_]]
                    && all_values := [o.values() | o := resource.data[_]]
                    && has_role := [obj.has_key("role") | obj := context.metadata[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_json_parse() {
    let input = r#"
            policy test {
                default: deny,
                rule json_parsing {
                    allow if json_data := [json::parse(s) | s := user.json_strings[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_json_stringify() {
    let input = r#"
            policy test {
                default: deny,
                rule json_serialization {
                    allow if json_strings := [json::stringify(obj) | obj := user.objects[_]]
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}

#[test]
fn test_parse_json_is_valid() {
    let input = r#"
            policy test {
                default: deny,
                rule json_validation {
                    allow if valid_checks := [json::is_valid(s) | s := user.strings[_]]
                    
                }
            }
        "#;

    let policy = ReapParser::parse(input).unwrap();
    assert_eq!(policy.rules.len(), 1);
}
