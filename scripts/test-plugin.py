#!/usr/bin/env python3
"""zsh 插件 PTY 回归测试：成功 / 失败 / 早亡三径，断言 busy 复位、临时目录清理、动画停帧。

用法：scripts/test-plugin.py [插件路径]（默认仓库内 zsh/ask-opencode.plugin.zsh）

早亡径是本脚本存在的理由（#47）：完成子进程在写 OK/ERR 前被 SIGKILL 时，zsh 侧必须收尾——
busy 复位、临时目录删除、动画进程停帧；否则 busy 永久卡死、tmpdir 泄漏、动画进程残留。
成功/失败两径守卫「完成 FIFO 的 OK/ERR 单条契约」不回归。

驱动方式：用 pty.fork 起交互式 zsh（zle 激活）source 插件，向 tty 输入请求 + Tab 触发
widget，再向 tty 打字探测内部状态。zle 空闲时也在轮询 zle -F 的 fd，handler 触发后自行
重绘显示 zle -M 消息，无需额外按键。
"""
import os
import pty
import re
import select
import shutil
import sys
import tempfile
import time

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PLUGIN = sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO_ROOT, 'zsh', 'ask-opencode.plugin.zsh')

# 三份 fake ask-opencode 只差 generate 臂：分别模拟成功回填 / 失败报错 / 早亡
# （写信号前 SIGKILL 掉 wrapper）
def _fake(generate_body):
    return ('#!/bin/sh\n'
            'case "$1" in\n'
            '  generate) %s ;;\n'
            '  select)   printf \'echo demo\\n\' ;;\n'
            'esac\n') % generate_body

FAKES = {
    'ok': _fake(r'printf \'["echo demo"]\n\''),
    'err': _fake('echo "boom: 模型不可用" >&2; exit 1'),
    'dead': _fake('echo "$$ $PPID" > "$KILL_PID_FILE"; kill -9 "$PPID"'),
}

# 需镜像插件 _ask_opencode_animate 的 frames（zsh/ask-opencode.plugin.zsh）；改动画帧时同步更新
SPINNERS = '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'


def run_case(base, case):
    pidfile = os.path.join(base, 'pids-%s.txt' % case)
    bindir = os.path.join(base, 'bin-%s' % case)
    os.makedirs(bindir)
    fake = os.path.join(bindir, 'ask-opencode')
    with open(fake, 'w') as f:
        f.write(FAKES[case])
    os.chmod(fake, 0o755)

    pid, fd = pty.fork()
    if pid == 0:
        os.environ['TERM'] = 'xterm'
        os.environ['PATH'] = bindir + ':' + os.environ['PATH']
        os.environ['TMPDIR'] = base
        os.environ['KILL_PID_FILE'] = pidfile
        os.execvp('zsh', ['zsh', '-f', '-i'])

    def send(s):
        os.write(fd, s.encode())

    def recv(timeout=1.5, maxbytes=131072):
        end = time.time() + timeout
        buf = b''
        while time.time() < end:
            r, _, _ = select.select([fd], [], [], 0.2)
            if r:
                try:
                    data = os.read(fd, 4096)
                except OSError:
                    break
                if not data:
                    break
                buf += data
                if len(buf) >= maxbytes:
                    break
        return buf.decode(errors='replace')

    try:
        time.sleep(0.5)
        recv(0.4)  # drain 启动输出
        send('source %s\n' % PLUGIN)
        recv(0.6)
        send('# 测试请求')
        time.sleep(0.2)
        send('\t')
        time.sleep(1.0)
        out = recv(2.5)  # 完整捕获动画帧 + 最终 zle -M 消息 + 重绘

        if case == 'dead':
            for _ in range(30):
                if os.path.exists(pidfile):
                    break
                time.sleep(0.1)
            time.sleep(0.6)
            extra = recv(1.0)  # 若动画进程还在跑，这里会出现新的 spinner 帧
            assert not any(c in extra for c in SPINNERS), '早亡后动画进程仍在刷帧: %r' % extra[-300:]

        send('\x15')  # Ctrl-U 清空当前行（ok 径 BUFFER 已被回填成 echo demo）
        time.sleep(0.3)
        recv(0.6)

        send('echo "STATE busy=$_ask_opencode_busy tmpdir=$_ask_opencode_tmpdir"\n')
        m = re.search(r'STATE busy=([01]) tmpdir=(\S*)', recv(2.0))
        busy, tmpdir = (m.group(1), m.group(2)) if m else ('?', '?')
        residue = [f for f in os.listdir(base) if f.startswith('ask-opencode.')]

        label = {'ok': '成功回填', 'err': '失败提示', 'dead': '早亡收尾'}[case]
        if case == 'ok':
            assert '已回填' in out, 'ok: 未见回填提示'
            assert 'echo demo' in out, 'ok: 未见回填内容'
        elif case == 'err':
            assert 'boom' in out, 'err: 未见错误提示'
        else:
            assert '未收到完成信号' in out, 'dead: 未见早亡提示'
        assert busy == '0', '%s: busy 未复位 (=%s)' % (label, busy)
        assert tmpdir == '', '%s: 临时目录未清 (=%s)' % (label, tmpdir)
        assert not residue, '%s: 磁盘残留临时目录 %r' % (label, residue)
        print('  [%s] PASS' % label)
    finally:
        send('exit\n')
        time.sleep(0.2)
        try:
            os.kill(pid, 9)
        except ProcessLookupError:
            pass
        os.close(fd)


def main():
    print('zsh 插件 PTY 回归测试（插件: %s）' % PLUGIN)
    base = tempfile.mkdtemp(prefix='ask-opencode-plugin-test.')
    try:
        for case in FAKES:
            run_case(base, case)
    finally:
        shutil.rmtree(base, ignore_errors=True)
    print('全部通过')


if __name__ == '__main__':
    main()
