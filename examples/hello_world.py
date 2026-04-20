import drun
import textwrap


def main():
    code_to_run = textwrap.dedent("""
    import os
    with open('/workspace/examples/hello.txt', 'a') as file:
        file.write('\\nHello from WASM!')
    """)
    drun.execute(code_to_run, mounts=['examples/hello.txt'], interactive=True)


if __name__ == '__main__':
    main()
