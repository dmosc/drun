import drun
import textwrap


def main():
    code_to_run = textwrap.dedent("""
    import os
    with open('/workspace/hello.txt', 'a') as file:
        file.write('\\nHello from WASM!')
    """)
    print(code_to_run)
    output = drun.execute(code_to_run, mounts=['examples/hello.txt'])
    print(output.stdout, output.files)


if __name__ == '__main__':
    main()
