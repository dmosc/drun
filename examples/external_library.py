import drun

code = """
import micropip
await micropip.install('faker')

from faker import Faker
import json

fake = Faker()
records = [
    {'name': fake.name(), 'email': fake.email(), 'address': fake.address()}
    for _ in range(5)
]

with open('/workspace/records.json', 'w') as f:
    json.dump(records, f, indent=2)

print(f"Generated {len(records)} records")
"""

result = drun.execute(code)
print(result.stdout)
