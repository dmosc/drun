import drun

session = drun.Session()

session.execute(
    'with open("/workspace/state.txt", "w") as f: f.write("step 1")')
session.execute(
    'with open("/workspace/state.txt", "w") as f: f.write("step 2")')
session.execute('open("/workspace/state.txt").read()')
session.rollback(0)
session.execute('open("/workspace/state.txt").read()')
print(session.current.id)
