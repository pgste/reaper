#!/usr/bin/env python3
"""
Generate test data for Trading Desk Access Control
Creates 1000 traders, desks, and books with realistic mappings
"""

import json
import random
from typing import List, Dict, Any

# Configuration
NUM_TRADERS = 1000
NUM_DESKS = 10
BOOKS_PER_DESK = 50  # 500 total books

# Asset types available in the trading system
ASSET_TYPES = ["equity", "fx", "rates", "credit", "commodities", "derivatives", "structured"]

# Desk names
DESK_NAMES = [
    "equity_us", "equity_eu", "equity_asia",
    "fx_g10", "fx_em",
    "rates_govies", "rates_swaps",
    "credit_ig", "credit_hy",
    "commodities"
]

# Role distribution
ROLES = {
    "trader": 0.80,        # 80% are traders
    "desk_head": 0.05,     # 5% are desk heads
    "risk_manager": 0.08,  # 8% are risk managers
    "compliance": 0.05,    # 5% are compliance
    "analyst": 0.02        # 2% are analysts (read-only)
}

# Status distribution
STATUSES = {
    "active": 0.92,
    "inactive": 0.05,
    "suspended": 0.03
}

def weighted_choice(choices: Dict[str, float]) -> str:
    """Select a random item based on weights"""
    items = list(choices.keys())
    weights = list(choices.values())
    return random.choices(items, weights=weights)[0]

def generate_desks() -> List[Dict[str, Any]]:
    """Generate desk entities"""
    desks = []
    for i, name in enumerate(DESK_NAMES):
        # Assign 2-4 asset types to each desk
        num_assets = random.randint(2, 4)
        desk_assets = random.sample(ASSET_TYPES, num_assets)

        desks.append({
            "id": f"desk_{i}",
            "type": "Desk",
            "attributes": {
                "name": name,
                "asset_types": desk_assets,
                "region": name.split("_")[-1] if "_" in name else "global",
                "status": "active"
            }
        })
    return desks

def generate_books(desks: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Generate book entities for each desk"""
    books = []
    book_id = 0

    for desk in desks:
        desk_id = desk["id"]
        desk_assets = desk["attributes"]["asset_types"]
        desk_name = desk["attributes"]["name"]

        for j in range(BOOKS_PER_DESK):
            # Each book has one asset type from the desk's allowed types
            asset_type = random.choice(desk_assets)

            books.append({
                "id": f"book_{book_id}",
                "type": "Book",
                "attributes": {
                    "name": f"{desk_name}_book_{j}",
                    "asset_type": asset_type,
                    "desk_id": desk_id,
                    "currency": random.choice(["USD", "EUR", "GBP", "JPY", "CHF"]),
                    "status": "active" if random.random() > 0.05 else "closed"
                }
            })
            book_id += 1

    return books

def generate_traders(desks: List[Dict[str, Any]], books: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Generate trader entities with desk and book mappings"""
    traders = []

    # Group books by desk for efficient assignment
    books_by_desk = {}
    for book in books:
        desk_id = book["attributes"]["desk_id"]
        if desk_id not in books_by_desk:
            books_by_desk[desk_id] = []
        books_by_desk[desk_id].append(book["id"])

    for i in range(NUM_TRADERS):
        role = weighted_choice(ROLES)
        status = weighted_choice(STATUSES)

        # Determine desk access based on role
        if role == "risk_manager" or role == "compliance":
            # Risk and compliance can access all desks
            desk_ids = [d["id"] for d in desks]
        elif role == "desk_head":
            # Desk heads manage 1-2 desks
            num_desks = random.randint(1, 2)
            desk_ids = [d["id"] for d in random.sample(desks, num_desks)]
        else:
            # Regular traders access 1-3 desks
            num_desks = random.randint(1, 3)
            desk_ids = [d["id"] for d in random.sample(desks, num_desks)]

        # Assign books based on desk access
        assigned_books = []
        for desk_id in desk_ids:
            if desk_id in books_by_desk:
                # Assign 5-20 books per desk for traders
                num_books = min(random.randint(5, 20), len(books_by_desk[desk_id]))
                assigned_books.extend(random.sample(books_by_desk[desk_id], num_books))

        # Generate realistic names
        first_names = ["James", "John", "Robert", "Michael", "David", "William", "Richard",
                       "Joseph", "Thomas", "Charles", "Emma", "Olivia", "Sophia", "Isabella",
                       "Mia", "Charlotte", "Amelia", "Harper", "Evelyn", "Abigail"]
        last_names = ["Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia", "Miller",
                      "Davis", "Rodriguez", "Martinez", "Hernandez", "Lopez", "Gonzalez",
                      "Wilson", "Anderson", "Thomas", "Taylor", "Moore", "Jackson", "Martin"]

        first = random.choice(first_names)
        last = random.choice(last_names)

        traders.append({
            "id": f"trader_{i}",
            "type": "User",
            "attributes": {
                "name": f"{first} {last}",
                "email": f"{first.lower()}.{last.lower()}{i}@tradingfirm.com",
                "role": role,
                "status": status,
                "desk_ids": desk_ids,
                "book_ids": assigned_books,
                "employee_id": f"EMP{str(i).zfill(5)}",
                "department": "trading"
            }
        })

    return traders

def generate_trader_book_mappings(traders: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Generate explicit trader-to-book mapping entities"""
    mappings = []

    for trader in traders:
        mappings.append({
            "id": f"mapping_{trader['id']}",
            "type": "TraderBookMapping",
            "attributes": {
                "trader_id": trader["id"],
                "book_ids": trader["attributes"]["book_ids"]
            }
        })

    return mappings

def main():
    print("Generating trading test data...")

    # Generate entities
    desks = generate_desks()
    print(f"  Created {len(desks)} desks")

    books = generate_books(desks)
    print(f"  Created {len(books)} books")

    traders = generate_traders(desks, books)
    print(f"  Created {len(traders)} traders")

    # Statistics
    active_traders = sum(1 for t in traders if t["attributes"]["status"] == "active")
    role_counts = {}
    for t in traders:
        role = t["attributes"]["role"]
        role_counts[role] = role_counts.get(role, 0) + 1

    print(f"\n  Active traders: {active_traders}")
    print(f"  Role distribution:")
    for role, count in sorted(role_counts.items()):
        print(f"    {role}: {count}")

    # Combine all entities
    all_entities = desks + books + traders

    output = {"entities": all_entities}

    # Write to file
    output_path = "services/reaper-bench/data/trading/trading_entities.json"
    with open(output_path, "w") as f:
        json.dump(output, f, indent=2)

    print(f"\n  Written to {output_path}")
    print(f"  Total entities: {len(all_entities)}")

    # Also create a compact version for faster loading
    compact_path = "services/reaper-bench/data/trading/trading_entities_compact.json"
    with open(compact_path, "w") as f:
        json.dump(output, f, separators=(",", ":"))

    print(f"  Compact version: {compact_path}")

    # Create sample test requests
    create_test_requests(traders, books)

def create_test_requests(traders: List[Dict[str, Any]], books: List[Dict[str, Any]]):
    """Create sample test requests for benchmarking"""
    requests = []

    # Mix of allowed and denied scenarios
    for i in range(100):
        trader = random.choice(traders)
        book = random.choice(books)
        action = random.choice(["view", "trade"])

        # Determine if this should be allowed
        trader_desk_ids = trader["attributes"]["desk_ids"]
        book_desk_id = book["attributes"]["desk_id"]
        trader_status = trader["attributes"]["status"]
        trader_role = trader["attributes"]["role"]

        expected = "deny"
        if trader_status == "active":
            if book_desk_id in trader_desk_ids:
                if action == "view":
                    expected = "allow"
                elif action == "trade" and trader_role in ["trader", "desk_head"]:
                    expected = "allow"
            if trader_role in ["risk_manager", "compliance"] and action == "view":
                expected = "allow"

        requests.append({
            "principal": trader["id"],
            "action": action,
            "resource": book["id"],
            "resource_desk_id": book_desk_id,
            "expected": expected
        })

    # Write test requests
    requests_path = "services/reaper-bench/data/trading/test_requests.json"
    with open(requests_path, "w") as f:
        json.dump(requests, f, indent=2)

    print(f"  Test requests: {requests_path}")

if __name__ == "__main__":
    main()
