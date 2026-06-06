import drun

session = drun.Session()

session.execute(
    "with open('/workspace/state.txt', 'w') as f: f.write('step 1')")
step1 = session.execute("print(open('/workspace/state.txt').read())")
print(f"[checkpoint {step1.id}] {step1.stdout}")

session.execute(
    "with open('/workspace/state.txt', 'w') as f: f.write('step 2')")
step2 = session.execute("print(open('/workspace/state.txt').read())")
print(f"[checkpoint {step2.id}] {step2.stdout}")

session.rollback(step1.id)

after_rollback = session.execute("print(open('/workspace/state.txt').read())")
print(
    f"[checkpoint {after_rollback.id}, after rollback] {after_rollback.stdout}")
