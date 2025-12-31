// Data Generator for OPA vs Reaper Benchmarks
// Generates 100k+ entity datasets for all policy scenarios

use clap::{Parser, Subcommand};
use rand::Rng;
use rand::seq::SliceRandom;
use serde_json::{json, Value};
use std::fs::File;
use std::io::Write;

#[derive(Parser)]
#[command(name = "generate-data")]
#[command(about = "Generate test data for OPA vs Reaper benchmarks")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Number of entities to generate
    #[arg(short, long, default_value = "100000")]
    count: usize,

    /// Output file path
    #[arg(short, long)]
    output: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate math policy data
    Math,
    /// Generate regex policy data
    Regex,
    /// Generate time policy data
    Time,
    /// Generate string policy data
    String,
    /// Generate collection policy data
    Collection,
    /// Generate comprehension policy data
    Comprehension,
    /// Generate JSON policy data
    Json,
    /// Generate mega policy data (all patterns)
    Mega,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    println!("Generating {} entities...", cli.count);

    let entities = match &cli.command {
        Commands::Math => generate_math_data(cli.count),
        Commands::Regex => generate_regex_data(cli.count),
        Commands::Time => generate_time_data(cli.count),
        Commands::String => generate_string_data(cli.count),
        Commands::Collection => generate_collection_data(cli.count),
        Commands::Comprehension => generate_comprehension_data(cli.count),
        Commands::Json => generate_json_data(cli.count),
        Commands::Mega => generate_mega_data(cli.count),
    };

    let output = json!({
        "entities": entities
    });

    let mut file = File::create(&cli.output)?;
    serde_json::to_writer_pretty(&mut file, &output)?;
    file.write_all(b"\n")?;

    println!("✓ Generated {} entities to {}", cli.count, cli.output);

    Ok(())
}

fn generate_math_data(count: usize) -> Vec<Value> {
    let mut rng = rand::thread_rng();
    let mut entities = Vec::new();

    for i in 0..count {
        let entity_type = i % 8; // 8 different math rules

        let mut inner_attrs = serde_json::Map::new();

        // Vary attributes based on rule being tested
        match entity_type {
            0 => {
                // Credit score
                inner_attrs.insert("credit_score".to_string(), json!(rng.gen_range(600..850)));
            }
            1 => {
                // Budget check
                inner_attrs.insert("order_total".to_string(), json!(rng.gen_range(50..500)));
                inner_attrs.insert("budget_limit".to_string(), json!(rng.gen_range(100..1000)));
            }
            2 => {
                // Rating
                inner_attrs.insert("average_rating".to_string(), json!(rng.gen_range(3.0..5.0)));
            }
            3 => {
                // Price
                inner_attrs.insert("list_price".to_string(), json!(rng.gen_range(1..10000)));
            }
            4 => {
                // Score/Tier
                inner_attrs.insert("score".to_string(), json!(rng.gen_range(40.0..100.0)));
            }
            5 => {
                // Temperature
                inner_attrs.insert("temperature".to_string(), json!(rng.gen_range(-50..50)));
            }
            6 => {
                // Loyalty points
                inner_attrs.insert("total_points".to_string(), json!(rng.gen_range(0..2000)));
            }
            7 => {
                // Discount
                inner_attrs.insert("discount_percentage".to_string(), json!(rng.gen_range(0..60)));
            }
            _ => unreachable!(),
        }

        entities.push(json!({
            "id": format!("math_user_{}", i),
            "type": "user",
            "attributes": inner_attrs
        }));
    }

    entities
}

fn generate_regex_data(count: usize) -> Vec<Value> {
    let mut rng = rand::thread_rng();
    let mut entities = Vec::new();

    let emails = vec![
        "user@example.com",
        "admin@company.co.uk",
        "test.user+tag@domain.org",
        "invalid@",
        "@invalid.com",
    ];

    let phones = vec![
        "+1 (555) 123-4567",
        "555-123-4567",
        "(555) 123-4567",
        "invalid",
        "12345",
    ];

    let urls = vec![
        "https://example.com",
        "http://test.com/path",
        "https://sub.domain.com/path?query=1",
        "invalid",
        "ftp://nothttp.com",
    ];

    let ips = vec![
        "192.168.1.1",
        "10.0.0.1",
        "255.255.255.255",
        "999.999.999.999",
        "invalid",
    ];

    let uuids = vec![
        "550e8400-e29b-41d4-a716-446655440000",
        "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
        "invalid-uuid",
        "123",
    ];

    for i in 0..count {
        entities.push(json!({
            "id": format!("regex_user_{}", i),
            "type": "user",
            "attributes": {
                "email": emails.choose(&mut rng).unwrap(),
                "phone": phones.choose(&mut rng).unwrap(),
                "url": urls.choose(&mut rng).unwrap(),
                "ip_address": ips.choose(&mut rng).unwrap(),
                "uuid": uuids.choose(&mut rng).unwrap(),
                "credit_card": format!("{:04}-{:04}-{:04}-{:04}",
                    rng.gen_range(1000..9999),
                    rng.gen_range(1000..9999),
                    rng.gen_range(1000..9999),
                    rng.gen_range(1000..9999)
                ),
                "has_redacted_ssn": rng.gen_bool(0.7),
                "has_valid_csv": rng.gen_bool(0.7),
                "has_valid_log": rng.gen_bool(0.7)
            }
        }));
    }

    entities
}

fn generate_time_data(count: usize) -> Vec<Value> {
    let mut rng = rand::thread_rng();
    let mut entities = Vec::new();

    let roles = vec!["employee", "operator", "event_planner", "contractor", "system", "audit_logger", "api_client", "archiver"];

    for i in 0..count {
        entities.push(json!({
            "id": format!("time_user_{}", i),
            "type": "user",
            "attributes": {
                "role": roles.choose(&mut rng).unwrap(),
                // Token expiration (nanoseconds)
                "token_expires_at": 1765180000000000000i64 + rng.gen_range(-10000000000000i64..10000000000000i64),
                // Business hours
                "work_start_time": 1765185000000000000i64 - rng.gen_range(0..5000000000000i64),
                "work_end_time": 1765185000000000000i64 + rng.gen_range(0..5000000000000i64),
                // Age (birthdate in seconds)
                "birthdate": rng.gen_range(900000000..1100000000),
                // Lease
                "lease_end_time": 1765180000000000000i64 + rng.gen_range(-5000000000000i64..10000000000000i64),
                // Session
                "session_start_time": 1765185000000000000i64 - rng.gen_range(0..1000000000000i64),
                "session_extension_ns": 3600000000000i64,
                // Event scheduling
                "event_scheduled_time": 1765180000000000000i64 + rng.gen_range(0..10000000000000i64),
                // Access grants
                "access_grant_start": 1765185000000000000i64 - rng.gen_range(0..5000000000000i64),
                "access_grant_end": 1765185000000000000i64 + rng.gen_range(0..5000000000000i64),
                // Additional time fields
                "subscription_expires": 1765180000000000000i64 + rng.gen_range(-2000000000000i64..8000000000000i64),
                "trial_ends": 1765180000000000000i64 + rng.gen_range(-1000000000000i64..3000000000000i64),
                "contract_start": 1765185000000000000i64 - rng.gen_range(0..3000000000000i64),
                "contract_end": 1765185000000000000i64 + rng.gen_range(0..7000000000000i64),
                "certification_expires": 1765180000000000000i64 + rng.gen_range(-1000000000000i64..5000000000000i64),
                "has_valid_timestamp": rng.gen_bool(0.8),
                "can_log_audit": rng.gen_bool(0.8),
                "last_request_time": 1765186000000000000i64 - rng.gen_range(0..2000000000000i64),
                "rate_limit_window_ns": 60000000000i64
            }
        }));
    }

    entities
}

fn generate_string_data(count: usize) -> Vec<Value> {
    let mut rng = rand::thread_rng();
    let mut entities = Vec::new();

    let names = vec!["John Doe", "JOHN DOE", "john doe", " John Doe ", "Jane Smith"];
    let codes = vec!["admin123", "ADMIN123", "manager456", "MANAGER456", "user789"];
    let emails = vec!["user@company.com", "admin@partner.com", "test@external.com", "gov@agency.gov", "mil@military.mil", "edu@university.edu"];
    let usernames = vec!["admin_john", "mgr_jane", "user_test", "test_user", "operator"];

    for i in 0..count {
        entities.push(json!({
            "id": format!("string_user_{}", i),
            "type": "user",
            "attributes": {
                "name": names.choose(&mut rng).unwrap(),
                "access_code": codes.choose(&mut rng).unwrap(),
                "email": emails.choose(&mut rng).unwrap(),
                "username": usernames.choose(&mut rng).unwrap(),
                "role": if rng.gen_bool(0.5) { " manager " } else { "manager" }
            }
        }));
    }

    entities
}

fn generate_collection_data(count: usize) -> Vec<Value> {
    let mut rng = rand::thread_rng();
    let mut entities = Vec::new();

    let permissions = vec!["read", "write", "delete", "admin", "execute"];
    let skills = vec!["rust", "python", "javascript", "go", "java", "kubernetes", "aws", "terraform"];
    let groups = vec!["engineering", "platform", "admin", "superadmin", "manager", "director"];
    let tags = vec!["public", "draft", "review", "internal", "confidential"];

    for i in 0..count {
        let mut inner_attrs = serde_json::Map::new();

        // Random permissions (1-4)
        let perm_count = rng.gen_range(1..=4);
        let user_perms: Vec<_> = permissions.choose_multiple(&mut rng, perm_count).cloned().collect();
        inner_attrs.insert("permissions".to_string(), json!(user_perms));

        // Random skills (2-7)
        let skill_count = rng.gen_range(2..=7);
        let user_skills: Vec<_> = skills.choose_multiple(&mut rng, skill_count).cloned().collect();
        inner_attrs.insert("skills".to_string(), json!(user_skills));

        // Random groups (1-3)
        let group_count = rng.gen_range(1..=3);
        let user_groups: Vec<_> = groups.choose_multiple(&mut rng, group_count).cloned().collect();
        inner_attrs.insert("groups".to_string(), json!(user_groups));

        // Random tags (1-3)
        let tag_count = rng.gen_range(1..=3);
        let user_tags: Vec<_> = tags.choose_multiple(&mut rng, tag_count).cloned().collect();
        inner_attrs.insert("tags".to_string(), json!(user_tags));

        // Roles
        inner_attrs.insert("roles".to_string(), json!(vec!["user", if rng.gen_bool(0.3) { "admin" } else { "viewer" }]));

        // Projects
        let projects: Vec<_> = (0..rng.gen_range(1..4))
            .map(|j| json!({"id": format!("proj_{}", j), "active": rng.gen_bool(0.8)}))
            .collect();
        inner_attrs.insert("projects".to_string(), json!(projects));

        // Accounts
        let accounts: Vec<_> = (0..rng.gen_range(1..3))
            .map(|j| json!({"id": format!("acct_{}", j), "active": rng.gen_bool(0.9)}))
            .collect();
        inner_attrs.insert("accounts".to_string(), json!(accounts));

        // Metadata
        let mut metadata = serde_json::Map::new();
        metadata.insert("name".to_string(), json!(format!("User {}", i)));
        if rng.gen_bool(0.8) {
            metadata.insert("email".to_string(), json!(format!("user{}@example.com", i)));
        }
        if rng.gen_bool(0.7) {
            metadata.insert("phone".to_string(), json!(format!("+1-555-{:04}", i % 10000)));
        }
        inner_attrs.insert("metadata".to_string(), json!(metadata));

        // Email addresses
        let email_addresses: Vec<_> = (0..rng.gen_range(1..3))
            .map(|j| json!({"email": format!("user{}@example{}.com", i, j), "verified": rng.gen_bool(0.7)}))
            .collect();
        inner_attrs.insert("email_addresses".to_string(), json!(email_addresses));

        // Departments
        let dept_count = rng.gen_range(1..3);
        let departments: Vec<_> = (0..dept_count)
            .map(|_| {
                let perm_count = rng.gen_range(1..4);
                let dept_perms: Vec<_> = permissions.choose_multiple(&mut rng, perm_count).cloned().collect();
                json!({"name": groups.choose(&mut rng).unwrap(), "permissions": dept_perms})
            })
            .collect();
        inner_attrs.insert("departments".to_string(), json!(departments));

        entities.push(json!({
            "id": format!("collection_user_{}", i),
            "type": "user",
            "attributes": inner_attrs
        }));
    }

    entities
}

fn generate_comprehension_data(count: usize) -> Vec<Value> {
    let mut rng = rand::thread_rng();
    let mut entities = Vec::new();

    let priorities = vec!["low", "medium", "high"];
    let strings = vec!["apple", "banana", "cherry", "date", "elderberry", "fig"];

    for i in 0..count {
        let mut inner_attrs = serde_json::Map::new();

        // Numbers
        let numbers: Vec<i32> = (0..10).map(|_| rng.gen_range(1..20)).collect();
        inner_attrs.insert("numbers".to_string(), json!(numbers));

        let values: Vec<i32> = (0..10).map(|_| rng.gen_range(1..100)).collect();
        inner_attrs.insert("values".to_string(), json!(values));

        let prices: Vec<i32> = (0..5).map(|_| rng.gen_range(10..200)).collect();
        inner_attrs.insert("prices".to_string(), json!(prices));

        // Items
        let items: Vec<_> = (0..5)
            .map(|j| json!({"id": j, "priority": priorities.choose(&mut rng).unwrap()}))
            .collect();
        inner_attrs.insert("items".to_string(), json!(items));

        // Records
        let records: Vec<_> = (0..5)
            .map(|j| json!({"id": format!("rec_{}", j), "value": rng.gen_range(1..100), "active": rng.gen_bool(0.6), "verified": rng.gen_bool(0.7)}))
            .collect();
        inner_attrs.insert("records".to_string(), json!(records));

        // Data with scores
        let data: Vec<_> = (0..5)
            .map(|_| json!({"score": rng.gen_range(70..100), "verified": rng.gen_bool(0.7)}))
            .collect();
        inner_attrs.insert("data".to_string(), json!(data));

        // Nested groups
        let nested: Vec<_> = (0..4)
            .map(|j| {
                let mut group = serde_json::Map::new();
                group.insert("id".to_string(), json!(format!("group_{}", j)));
                if rng.gen_bool(0.7) {
                    group.insert("items".to_string(), json!(vec![1, 2, 3]));
                }
                if rng.gen_bool(0.6) {
                    group.insert("members".to_string(), json!(vec!["a", "b"]));
                }
                json!(group)
            })
            .collect();
        inner_attrs.insert("nested".to_string(), json!(nested));

        // Strings
        let user_strings: Vec<_> = strings.choose_multiple(&mut rng, 3).cloned().collect();
        inner_attrs.insert("strings".to_string(), json!(user_strings));

        entities.push(json!({
            "id": format!("comp_user_{}", i),
            "type": "user",
            "attributes": inner_attrs
        }));
    }

    entities
}

fn generate_json_data(count: usize) -> Vec<Value> {
    let mut rng = rand::thread_rng();
    let mut entities = Vec::new();

    for i in 0..count {
        let mut inner_attrs = serde_json::Map::new();

        // Payload
        inner_attrs.insert("payload".to_string(), json!({"valid": rng.gen_bool(0.8), "data": "test"}));

        // Profile
        let mut profile = serde_json::Map::new();
        profile.insert("name".to_string(), json!(format!("User {}", i)));
        if rng.gen_bool(0.9) {
            profile.insert("email".to_string(), json!(format!("user{}@example.com", i)));
        }
        if rng.gen_bool(0.8) {
            profile.insert("phone".to_string(), json!(format!("+1-555-{:04}", i % 10000)));
        }
        if rng.gen_bool(0.7) {
            profile.insert("address".to_string(), json!(format!("{} Main St", i)));
        }
        inner_attrs.insert("profile".to_string(), json!(profile));

        // Payment
        let mut payment = serde_json::Map::new();
        if rng.gen_bool(0.8) {
            payment.insert("card".to_string(), json!({"number": "4111-1111-1111-1111"}));
        }
        if rng.gen_bool(0.7) {
            payment.insert("billing_address".to_string(), json!({"street": format!("{} Billing St", i), "city": "NYC"}));
        }
        inner_attrs.insert("payment".to_string(), json!(payment));

        // Order items
        let order_items: Vec<_> = (0..rng.gen_range(0..5))
            .map(|j| json!({"id": j, "name": format!("Item {}", j), "price": rng.gen_range(10..100)}))
            .collect();
        inner_attrs.insert("order_items".to_string(), json!(order_items));

        // Form data
        inner_attrs.insert("form_data".to_string(), json!({
            "name": format!("User {}", i),
            "age": rng.gen_range(18..80),
            "active": rng.gen_bool(0.8)
        }));

        inner_attrs.insert("name".to_string(), json!(format!("User {}", i)));
        inner_attrs.insert("name_length".to_string(), json!(format!("User {}", i).len()));
        inner_attrs.insert("age".to_string(), json!(rng.gen_range(18..80)));
        inner_attrs.insert("verified".to_string(), json!(rng.gen_bool(0.7)));

        // Address
        inner_attrs.insert("address".to_string(), json!({
            "street": format!("{} Main St", i),
            "city": "New York",
            "zip": "10001"
        }));

        // Primary/Secondary data
        inner_attrs.insert("primary_data".to_string(), json!({
            "name": format!("User {}", i),
            "email": format!("user{}@example.com", i)
        }));

        inner_attrs.insert("secondary_data".to_string(), json!({
            "phone": format!("+1-555-{:04}", i % 10000),
            "address": format!("{} Main St", i)
        }));

        entities.push(json!({
            "id": format!("json_user_{}", i),
            "type": "user",
            "attributes": inner_attrs
        }));
    }

    entities
}

fn generate_mega_data(count: usize) -> Vec<Value> {
    let mut rng = rand::thread_rng();
    let mut entities = Vec::new();

    // Combine all generators for comprehensive testing
    let roles = vec!["employee", "operator", "event_planner", "contractor", "admin", "manager", "viewer"];
    let permissions = vec!["read", "write", "delete", "admin", "execute"];
    let skills = vec!["rust", "python", "javascript", "go", "java"];
    let groups = vec!["engineering", "platform", "admin", "superadmin", "manager"];

    for i in 0..count {
        let mut inner_attrs = serde_json::Map::new();
        inner_attrs.insert("role".to_string(), json!(roles.choose(&mut rng).unwrap()));

        // Math attributes
        inner_attrs.insert("credit_score".to_string(), json!(rng.gen_range(600..850)));
        inner_attrs.insert("order_total".to_string(), json!(rng.gen_range(50..500)));
        inner_attrs.insert("budget_limit".to_string(), json!(rng.gen_range(100..1000)));
        inner_attrs.insert("average_rating".to_string(), json!(rng.gen_range(3.0..5.0)));
        inner_attrs.insert("list_price".to_string(), json!(rng.gen_range(1..10000)));
        inner_attrs.insert("score".to_string(), json!(rng.gen_range(40.0..100.0)));
        inner_attrs.insert("temperature".to_string(), json!(rng.gen_range(-50..50)));
        inner_attrs.insert("total_points".to_string(), json!(rng.gen_range(0..2000)));
        inner_attrs.insert("discount_percentage".to_string(), json!(rng.gen_range(0..60)));

        // String attributes
        let names = vec!["admin", "manager", "user"];
        inner_attrs.insert("name".to_string(), json!(names.choose(&mut rng).unwrap()));
        inner_attrs.insert("access_code".to_string(), json!(format!("CODE{}", rng.gen_range(100..999))));
        inner_attrs.insert("email".to_string(), json!(format!("user{}@company.com", i)));
        inner_attrs.insert("username".to_string(), json!(format!("admin_{}", i)));

        // Regex attributes
        inner_attrs.insert("phone".to_string(), json!(format!("+1 (555) {:03}-{:04}", rng.gen_range(100..999), rng.gen_range(1000..9999))));
        inner_attrs.insert("url".to_string(), json!(format!("https://example{}.com", i)));
        inner_attrs.insert("ip_address".to_string(), json!(format!("192.168.{}.{}", i % 256, rng.gen_range(1..255))));
        inner_attrs.insert("uuid".to_string(), json!(uuid::Uuid::new_v4().to_string()));

        // Time attributes
        inner_attrs.insert("token_expires_at".to_string(), json!(1765180000000000000i64 + rng.gen_range(-5000000000000i64..10000000000000i64)));
        inner_attrs.insert("work_start_time".to_string(), json!(1765185000000000000i64 - rng.gen_range(0..3000000000000i64)));
        inner_attrs.insert("work_end_time".to_string(), json!(1765185000000000000i64 + rng.gen_range(0..3000000000000i64)));
        inner_attrs.insert("birthdate".to_string(), json!(rng.gen_range(900000000..1100000000)));
        inner_attrs.insert("subscription_expires".to_string(), json!(1765180000000000000i64 + rng.gen_range(-2000000000000i64..8000000000000i64)));

        // Collection attributes
        let perm_count = rng.gen_range(1..4);
        let user_perms: Vec<_> = permissions.choose_multiple(&mut rng, perm_count).cloned().collect();
        inner_attrs.insert("permissions".to_string(), json!(user_perms));

        let skill_count = rng.gen_range(2..6);
        let user_skills: Vec<_> = skills.choose_multiple(&mut rng, skill_count).cloned().collect();
        inner_attrs.insert("skills".to_string(), json!(user_skills));

        let group_count = rng.gen_range(1..3);
        let user_groups: Vec<_> = groups.choose_multiple(&mut rng, group_count).cloned().collect();
        inner_attrs.insert("groups".to_string(), json!(user_groups));

        // Comprehension attributes
        let numbers: Vec<i32> = (0..10).map(|_| rng.gen_range(1..20)).collect();
        inner_attrs.insert("numbers".to_string(), json!(numbers));

        let items: Vec<_> = (0..5)
            .map(|j| json!({"id": j, "priority": if rng.gen_bool(0.5) { "high" } else { "medium" }}))
            .collect();
        inner_attrs.insert("items".to_string(), json!(items));

        let records: Vec<_> = (0..5)
            .map(|j| json!({"id": format!("rec_{}", j), "value": rng.gen_range(1..100), "active": rng.gen_bool(0.6), "verified": rng.gen_bool(0.7)}))
            .collect();
        inner_attrs.insert("records".to_string(), json!(records));

        // JSON attributes
        inner_attrs.insert("payload".to_string(), json!({"valid": rng.gen_bool(0.8)}));
        inner_attrs.insert("profile".to_string(), json!({
            "name": format!("User {}", i),
            "email": format!("user{}@example.com", i),
            "phone": format!("+1-555-{:04}", i % 10000)
        }));

        entities.push(json!({
            "id": format!("mega_user_{}", i),
            "type": "user",
            "attributes": inner_attrs
        }));
    }

    entities
}
