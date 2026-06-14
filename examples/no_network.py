import drun

session = drun.Session(allowed_hosts=[])

result = session.execute("""
from pyodide.http import pyfetch
try:
    await pyfetch("https://example.com")
    print("connected")
except Exception as e:
    print(f"network blocked: {e}")
""")

print(result.stdout)

session = drun.Session(allowed_hosts=["*"])

result = session.execute("""
from pyodide.http import pyfetch
try:
    await pyfetch("https://example.com")
    print("connected")
except Exception as e:
    print(f"network blocked: {e}")
""")

print(result.stdout)
