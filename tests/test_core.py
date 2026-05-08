import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from cxs import core

class CoreTests(unittest.TestCase):
    def setUp(self):
        self.old_path = os.environ.get('PATH', '')
        os.environ['PATH'] = str(ROOT/'tests'/'fakebin') + os.pathsep + self.old_path
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root/'src').mkdir()
        (self.root/'docs').mkdir()
        (self.root/'data').mkdir()
        (self.root/'resources'/'code'/'github').mkdir(parents=True)
        (self.root/'experiments'/'runs'/'r1').mkdir(parents=True)
        (self.root/'src'/'app.py').write_text('def train_loop(x):\n    return x\n\nclass Trainer:\n    pass\n', encoding='utf-8')
        (self.root/'docs'/'README.md').write_text('hello\nneedle ' + 'x'*1000 + '\n', encoding='utf-8')
        (self.root/'resources'/'code'/'github'/'vendor.txt').write_text('needle vendor\n', encoding='utf-8')
        with (self.root/'data'/'records.jsonl').open('w', encoding='utf-8') as f:
            f.write(json.dumps({'doc_id':'doc-1','token_count':12,'text':'A'*10000,'kind':'summary'})+'\n')
            f.write(json.dumps({'doc_id':'doc-2','token_count':99,'text':'needle hidden in large text','kind':'raw'})+'\n')
        (self.root/'experiments'/'runs'/'r1'/'metrics.jsonl').write_text(json.dumps({'run_id':'r1','acc':0.9})+'\n', encoding='utf-8')
    def tearDown(self):
        os.environ['PATH'] = self.old_path
        self.tmp.cleanup()
    def test_find_budget_and_excludes(self):
        res = core.cxs_find(query='needle', root=str(self.root), scopes=['docs'], max_line_chars=60)
        self.assertTrue(res['hits'])
        self.assertLessEqual(len(res['hits'][0]['snippet']), 80)
        res2 = core.cxs_find(query='vendor', root=str(self.root), scopes=['docs','src'])
        self.assertEqual(res2['hits'], [])
    def test_find_defaults_come_from_budget(self):
        (self.root/'src'/'budget.py').write_text(''.join('budget-key\n' for _ in range(25)), encoding='utf-8')
        res = core.cxs_find(query='budget-key', root=str(self.root), paths=['src/budget.py'])
        self.assertEqual(len(res['hits']), 25)
        self.assertLessEqual(len(res['hits']), core.DEFAULT_BUDGET.max_hits_per_file)
    def test_find_files_only(self):
        (self.root/'src'/'multi.py').write_text('alpha\nbeta\n', encoding='utf-8')
        (self.root/'src'/'one.py').write_text('alpha\n', encoding='utf-8')
        res = core.cxs_find(query='needle', root=str(self.root), scopes=['docs'], files_only=True)
        self.assertTrue(res['files_only'])
        self.assertIn({'file': 'docs/README.md'}, res['files'])
        self.assertNotIn('hits', res)
        self.assertEqual(res['next'][0]['op'], 'find')
        res2 = core.cxs_find(terms=['alpha','beta'], match='all', root=str(self.root), scopes=['src'], files_only=True)
        self.assertIn({'file': 'src/multi.py'}, res2['files'])
        self.assertNotIn({'file': 'src/one.py'}, res2['files'])
    def test_find_files_only_pagination_and_next_meta(self):
        for i in range(4):
            (self.root/'src'/f'page{i}.py').write_text('page-key\n', encoding='utf-8')
        res = core.cxs_find(query='page-key', root=str(self.root), scopes=['src'], files_only=True, max_total_hits=2)
        self.assertEqual(len(res['files']), 2)
        self.assertEqual(res['status'], 'truncated')
        self.assertTrue(res['stats']['has_more'])
        self.assertEqual(res['next_page']['offset'], 2)
        self.assertIn('next_meta', res)
        res2 = core.cxs_find(query='page-key', root=str(self.root), scopes=['src'], files_only=True, max_total_hits=2, offset=res['next_page']['offset'])
        self.assertEqual(len(res2['files']), 2)
        self.assertNotEqual(res['files'], res2['files'])
    def test_find_single_explicit_file_has_lines(self):
        res = core.cxs_find(query='return', root=str(self.root), paths=['src/app.py'])
        self.assertEqual(len(res['hits']), 1)
        self.assertEqual(res['hits'][0]['file'], 'src/app.py')
        self.assertEqual(res['hits'][0]['line'], 2)
        self.assertNotIn('next', res)
    def test_find_reports_output_and_per_file_limits(self):
        (self.root/'src'/'many.py').write_text(''.join('needle ' + 'x'*200 + '\n' for _ in range(10)), encoding='utf-8')
        res = core.cxs_find(query='needle', root=str(self.root), paths=['src/many.py'], max_hits_per_file=3)
        self.assertEqual(res['status'], 'truncated')
        self.assertIn('max_hits_per_file', res['stats']['limit_reasons'])
        self.assertEqual(res['stats']['files_at_hit_limit'], 1)
        self.assertEqual(res['limited_files'][0]['file'], 'src/many.py')
        res2 = core.cxs_find(query='needle', root=str(self.root), paths=['src/many.py'], max_hits_per_file=20, max_line_chars=240, max_total_chars=1000)
        self.assertEqual(res2['status'], 'truncated')
        self.assertIn('max_total_chars', res2['stats']['limit_reasons'])
    def test_missing_path_does_not_fallback(self):
        res = core.cxs_find(query='needle', root=str(self.root), paths=['missing.md'])
        self.assertEqual(res['status'], 'error')
        self.assertEqual(res['error'], 'no_valid_paths')
    def test_explicit_run_file_allowed(self):
        rel = 'experiments/runs/r1/metrics.jsonl'
        res = core.cxs_find(query='r1', root=str(self.root), paths=[rel])
        self.assertEqual(len(res['hits']), 1)
        js = core.cxs_json(query='r1', root=str(self.root), paths=[rel])
        self.assertEqual(len(js['hits']), 1)
    def test_files(self):
        res = core.cxs_files(query='app', root=str(self.root), scopes=['src'])
        self.assertTrue(any(x['file'] == 'src/app.py' for x in res['files']))
    def test_read_op_removed(self):
        self.assertFalse(hasattr(core, 'cxs_read'))
    def test_symbol(self):
        res = core.cxs_symbol('train_loop', root=str(self.root))
        self.assertTrue(any('def train_loop' in h['snippet'] for h in res['hits']))
    def test_json_large_field_rules(self):
        res = core.cxs_json(query='needle hidden', root=str(self.root), paths=['data/records.jsonl'])
        self.assertEqual(res['hits'], [])
        res2 = core.cxs_json(query='needle hidden', root=str(self.root), paths=['data/records.jsonl'], search_large_fields=True)
        self.assertEqual(len(res2['hits']), 1)
        res3 = core.cxs_json(filters={'doc_id':'doc-1'}, fields=['doc_id','token_count','text'], root=str(self.root), paths=['data/records.jsonl'])
        self.assertEqual(res3['hits'][0]['record']['doc_id'], 'doc-1')
        self.assertLessEqual(len(res3['hits'][0]['record']['text']), 320)
    def test_runs_scope_default(self):
        res = core.cxs_json(query='r1', root=str(self.root), scopes=['analysis'])
        self.assertEqual(res['hits'], [])
        res2 = core.cxs_json(query='r1', root=str(self.root), scopes=['runs'])
        self.assertEqual(len(res2['hits']), 1)
    def test_cli_aliases_path_and_scope_validation(self):
        cmd = [
            sys.executable, '-m', 'cxs.cli',
            'find', 'needle',
            '--root', str(self.root),
            '--path', 'docs/README.md',
            '--max-total-hits', '1',
            '--max-hits-per-file', '1',
            '--max-line-chars', '80',
            '--max-total-chars', '1000',
        ]
        proc = subprocess.run(cmd, cwd=str(ROOT), text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        res = json.loads(proc.stdout)
        self.assertEqual(len(res['hits']), 1)
        self.assertEqual(res['hits'][0]['file'], 'docs/README.md')

        bad_scope = subprocess.run(
            [
                sys.executable, '-m', 'cxs.cli',
                'find', 'needle',
                '--root', str(self.root),
                '--scope', 'README.md',
            ],
            cwd=str(ROOT),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self.assertNotEqual(bad_scope.returncode, 0)
        self.assertIn('invalid choice', bad_scope.stderr)
    def test_rg_shim_preserves_query(self):
        proc = subprocess.run(
            [
                sys.executable, str(ROOT/'bin'/'rg'),
                '-n', 'return', str(self.root/'src'/'app.py'),
            ],
            cwd=str(self.root),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        res = json.loads(proc.stdout)
        self.assertEqual(res['query'], 'return')
        self.assertEqual(res['terms'], ['return'])
        self.assertEqual(res['hits'][0]['file'], 'src/app.py')

class MCPTests(unittest.TestCase):
    def test_mcp_initialize_tools_list_and_terse(self):
        server = ROOT/'mcp'/'cxs_mcp_server.py'
        proc = subprocess.Popen([sys.executable, str(server)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        assert proc.stdin and proc.stdout
        proc.stdin.write(json.dumps({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-06-18','capabilities':{},'clientInfo':{'name':'t','version':'0'}}})+'\n')
        proc.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n')
        proc.stdin.write(json.dumps({'jsonrpc':'2.0','id':2,'method':'tools/list','params':{}})+'\n')
        proc.stdin.flush()
        r1 = json.loads(proc.stdout.readline())
        r2 = json.loads(proc.stdout.readline())
        proc.stdin.close(); proc.terminate()
        try: proc.wait(timeout=2)
        except Exception: proc.kill(); proc.wait(timeout=2)
        if proc.stdout: proc.stdout.close()
        if proc.stderr: proc.stderr.close()
        self.assertIn('find(files_only:true)', r1['result']['instructions'])
        text = json.dumps(r2, ensure_ascii=False)
        self.assertIn('\"cxs\"', text)
        self.assertNotIn('\"read\"', text)
        self.assertNotIn('cxs_find', text)
        self.assertLess(len(text), 900)  # keep tool-list injection bounded

    def test_mcp_call_single_tool(self):
        server = ROOT/'mcp'/'cxs_mcp_server.py'
        proc = subprocess.Popen([sys.executable, str(server)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        assert proc.stdin and proc.stdout
        proc.stdin.write(json.dumps({'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'cxs','arguments':{'op':'self_check','args':{'root':str(ROOT)}}}})+'\n')
        proc.stdin.flush()
        r = json.loads(proc.stdout.readline())
        proc.stdin.close(); proc.terminate()
        try: proc.wait(timeout=2)
        except Exception: proc.kill(); proc.wait(timeout=2)
        if proc.stdout: proc.stdout.close()
        if proc.stderr: proc.stderr.close()
        self.assertIn('content', r['result'])

if __name__ == '__main__':
    unittest.main()
