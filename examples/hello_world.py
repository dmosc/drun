import drun
import textwrap


def main():
    code_to_run = textwrap.dedent("""
    with open('/workspace/hello.txt', 'w') as file:
        file.write('Hello from WASM!')
    """)
    print(code_to_run)
    output = drun.execute(code_to_run)
    print(output)


if __name__ == '__main__':
    main()
