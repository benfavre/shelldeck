"""SDTEST-2824/2825 / SDUC-564: complete paged inventory and large Jcode metadata.

Disposable native stores only. No server, credentials or provider processes.
"""
import io
import json
import os
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'crates/shelldeck-core/src/agent_remote_discovery.py'


def inventory(root, cursor=None):
    output = io.BytesIO()
    stdout = io.TextIOWrapper(output, encoding='utf-8')
    argv = [str(SCRIPT)] + ([cursor] if cursor is not None else [])
    with patch.object(Path, 'home', return_value=root), patch.dict(os.environ, {}, clear=True), patch('sys.stdout', stdout), patch('sys.argv', argv):
        try:
            runpy.run_path(str(SCRIPT), run_name='__main__')
        except SystemExit as error:
            if error.code != 0:
                raise
        stdout.flush()
    return json.loads(output.getvalue())


class DiscoveryPages(unittest.TestCase):
    # SDTEST-2824 / SDUC-564
    def test_every_older_session_is_reachable_once_while_active_lists_change(self):
        with tempfile.TemporaryDirectory(prefix='shelldeck-discovery-pages-') as folder:
            root = Path(folder)
            directory = root / '.codex/sessions/2026/10/30'
            directory.mkdir(parents=True)
            for index in range(451):
                path = directory / f'rollout-session-{index:04}.jsonl'
                path.write_text(json.dumps({'type':'session_meta','payload':{'id':f'session-{index:04}','cwd':'/project'}})+'\n'+json.dumps({'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_text','text':f'Investigate issue {index}'}]}})+'\n')
                os.utime(path, (1000 + index, 1000 + index))
            first = inventory(root)
            self.assertEqual(len(first['sessions']), 200)
            self.assertTrue(first['next_cursor'])
            self.assertEqual(first['sessions'][0]['title'], 'Investigate issue 450')
            second = inventory(root, first['next_cursor'])
            third = inventory(root, second['next_cursor'])
            all_rows = first['sessions'] + second['sessions'] + third['sessions']
            self.assertEqual(len(all_rows), 451)
            self.assertEqual(len({row['session_id'] for row in all_rows}), 451)
            self.assertIsNone(third['next_cursor'])
            self.assertFalse(third['truncated'])
            self.assertEqual(third['sessions'][-1]['session_id'], 'session-0000')
            # Appending inside a file is harmless if candidate order is unchanged.
            recent = directory / 'rollout-session-0450.jsonl'
            with recent.open('a') as handle:
                handle.write(json.dumps({'type':'event_msg','payload':{'type':'token_count'}})+'\n')
            self.assertNotIn('error', inventory(root, first['next_cursor']))
            # Newer arrivals do not invalidate older pages or repeat already-seen rows.
            new = directory/'rollout-new.jsonl'
            new.write_text(json.dumps({'type':'session_meta','payload':{'id':'new','cwd':'/project'}})+'\n')
            continued = inventory(root, first['next_cursor'])
            self.assertEqual([row['session_id'] for row in continued['sessions']], [row['session_id'] for row in second['sessions']])
            self.assertEqual(inventory(root)['sessions'][0]['session_id'], 'new')
            # Same-shaped cursor from another source is never accepted.
            with tempfile.TemporaryDirectory(prefix='shelldeck-foreign-') as foreign:
                self.assertIn('Refresh', inventory(Path(foreign), first['next_cursor'])['error'])
            self.assertIn('Refresh', inventory(root, 'invalid_cursor')['error'])
            self.assertIn('Refresh', inventory(root, 'W10=')['error'])

    # SDTEST-2825 / SDUC-564
    def test_large_jcode_tail_metadata_and_nested_spoofs_are_handled_without_materializing_messages(self):
        with tempfile.TemporaryDirectory(prefix='shelldeck-large-native-') as folder:
            root = Path(folder)
            directory = root / '.jcode/sessions'
            directory.mkdir(parents=True)
            path = directory / 'session_large.json'
            path.write_text(json.dumps({
                'id':'session_large', 'title':'Long-running investigation',
                'messages':[
                    {'role':'user','content':[{'type':'text','text':'First prompt'}]},
                    {'role':'assistant','content':[{'type':'text','text':'x' * (9*1024*1024)}, {'type':'tool_result','content':{'id':'forged','working_dir':'/wrong','model':'wrong'}}]},
                ],
                'working_dir':'/srv/original', 'model':'deepseek-current',
                'updated_at':'2026-10-30T10:00:00Z', 'parent_id':None,
                'env_snapshots':{'nested':{'working_dir':'/also-wrong'}},
            }))
            child = directory / 'session_child.json'
            child.write_text(json.dumps({'id':'session_child','working_dir':'/srv/original','messages':[],'parent_id':'session_large'}))
            page = inventory(root)
            self.assertFalse(page['truncated'])
            self.assertEqual(page['warnings'], [])
            self.assertEqual(len(page['sessions']), 1)
            row = page['sessions'][0]
            self.assertEqual(row['session_id'],'session_large')
            self.assertEqual(row['workdir'],'/srv/original')
            self.assertEqual(row['model'],'deepseek-current')
            self.assertEqual(row['title'],'Long-running investigation')
            self.assertNotIn('messages', row)
            (directory/'broken.json').write_text('{\"id\":\"broken\"')
            retried = inventory(root)
            self.assertEqual(len(retried['sessions']), 1)
            self.assertTrue(retried['warnings'])


if __name__ == '__main__':
    unittest.main()
