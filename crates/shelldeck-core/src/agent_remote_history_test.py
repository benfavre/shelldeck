"""SDTEST-2815 / SDUC-563: synthetic native formats, complete paging and authority."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('history', Path(__file__).with_name('agent_remote_history.py'))
history = importlib.util.module_from_spec(spec)
spec.loader.exec_module(history)


class NativeHistoryTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.home = Path(self.temp.name)
        self.environment = patch.dict(os.environ, {'HOME': str(self.home), 'CODEX_HOME': str(self.home / '.codex'), 'CLAUDE_CONFIG_DIR': str(self.home / '.claude')})
        self.environment.start()

    def tearDown(self):
        self.environment.stop()
        self.temp.cleanup()

    def request(self, provider, **extra):
        return dict(provider=provider, session_id='native-id', workdir='/project', position='latest', **extra)

    def write(self, provider, rows):
        paths = {'codex': '.codex/sessions/2025/01/01/rollout-old-native-id.jsonl', 'claude': '.claude/projects/-project/native-id.jsonl', 'jcode': '.jcode/sessions/native-id.json'}
        path = self.home / paths[provider]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(rows) if provider == 'jcode' else '\n'.join(json.dumps(row, ensure_ascii=False) for row in rows) + '\n')
        return path

    def all_entries(self, request):
        page = history.run(request)
        self.assertLessEqual(len(page['entries']), 64)
        entries = page['entries']
        while page['start']:
            older = history.run(dict(request, position='before', boundary=page['start'], snapshot=page['snapshot']))
            self.assertEqual(older['end'], page['start'])
            # Newer navigation returns the exact next page at the requested boundary.
            newer = history.run(dict(request, position='after', boundary=older['end'], snapshot=page['snapshot']))
            self.assertEqual(newer['entries'], page['entries'][:64])
            entries = older['entries'] + entries
            page = older
        self.assertEqual([e['index'] for e in entries], list(range(len(entries))))
        return entries

    def test_codex_full_unicode_tools_and_snapshot_append(self):
        content = ('é🌍\n' * 20000)
        rows = [{'type':'session_meta','payload':{'id':'native-id','cwd':'/project'}}, {'type':'turn_context','payload':{'model':'model-codex'}}]
        for n in range(140):
            rows += [{'type':'event_msg','payload':{'type':'user_message','message':str(n)}}, {'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_text','text':str(n)}]}}]
        rows += [{'type':'response_item','payload':{'type':'message','role':'assistant','content':[{'type':'output_text','text':content}]}}, {'type':'response_item','payload':{'type':'function_call','name':'read_file','arguments':'{"path":"test"}'}}, {'type':'response_item','payload':{'type':'function_call_output','call_id':'call-1','output':'Result\ncomplete'}}, {'type':'response_item','payload':{'type':'reasoning','summary':[{'type':'summary_text','text':'Visible summary'}]}}]
        path = self.write('codex', rows)
        request = self.request('codex')
        entries = self.all_entries(request)
        self.assertEqual(len([e for e in entries if e['kind']=='user']),140)
        self.assertEqual(''.join(e['text'] for e in entries if e['kind']=='assistant'),content)
        self.assertEqual([e['text'] for e in entries if e['kind']=='tool'],['{"path":"test"}','Result\ncomplete'])
        page = history.run(request)
        self.assertEqual(page['model'],'model-codex')
        with path.open('a') as handle:
            handle.write(json.dumps({'type':'response_item','payload':{'type':'message','role':'assistant','content':[{'type':'output_text','text':'Appended'}]}})+'\n')
        self.assertEqual(history.run(dict(request,snapshot=page['snapshot']))['entries'],page['entries'])
        self.assertEqual(history.run(request)['total_entries'], page['total_entries']+1)
        path.write_text(path.read_text().replace('Visible summary','Changed summary'))
        with self.assertRaisesRegex(history.HistoryError,'changed'):
            history.run(dict(request,snapshot=page['snapshot']))


    def test_codex_mixed_legacy_turns_deduplicate_only_matching_records(self):
        rows = [{'type':'session_meta','payload':{'id':'native-id','cwd':'/project'}},
                {'type':'event_msg','payload':{'type':'user_message','message':'Legacy question'}},
                {'type':'event_msg','payload':{'type':'agent_message','message':'Legacy answer'}},
                {'type':'event_msg','payload':{'type':'agent_reasoning','text':'Legacy reasoning'}}]
        for turn in ('first', 'second'):
            rows += [{'type':'event_msg','payload':{'type':'task_started','turn_id':turn}},
                     {'type':'event_msg','payload':{'type':'user_message','message':'Same question'}},
                     {'type':'event_msg','payload':{'type':'token_count'}},
                     {'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_text','text':'Same question'}]}},
                     {'type':'response_item','payload':{'type':'message','role':'assistant','content':[{'type':'output_text','text':'Same answer'}]}},
                     {'type':'event_msg','payload':{'type':'token_count'}},
                     {'type':'event_msg','payload':{'type':'agent_message','message':'Same answer'}},
                     {'type':'event_msg','payload':{'type':'agent_reasoning','text':'Summary'}},
                     {'type':'response_item','payload':{'type':'reasoning','summary':[{'type':'summary_text','text':'Summary'}]}},
                     {'type':'event_msg','payload':{'type':'task_complete','turn_id':turn}}]
        rows += [{'type':'event_msg','payload':{'type':'task_started','turn_id':'event-only'}},
                 {'type':'event_msg','payload':{'type':'user_message','message':'Same question'}},
                 {'type':'event_msg','payload':{'type':'agent_message','message':'Same answer'}}]
        self.write('codex', rows)
        entries = self.all_entries(self.request('codex'))
        self.assertEqual([e['text'] for e in entries if e['kind']=='user'], ['Legacy question'] + ['Same question']*3)
        self.assertEqual([e['text'] for e in entries if e['kind']=='assistant'], ['Legacy answer'] + ['Same answer']*3)
        self.assertEqual([e['text'] for e in entries if e['kind']=='reasoning'], ['Legacy reasoning','Summary','Summary'])

    def test_jcode_large_file_streamed_and_all_messages(self):
        # >8MiB, with identity/cwd AFTER messages, as native Jcode stores them.
        messages = [{'role':'user','content':[{'type':'text','text':'question'}]}]
        messages += [{'role':'assistant','content':[{'type':'text','text':str(n)+':'+('λ'*20000)}]} for n in range(240)]
        messages += [{'role':'assistant','content':[{'type':'reasoning_trace','text':'Reason'},{'type':'open_a_i_reasoning','summary':[{'type':'summary_text','text':'Saved summary'}],'encrypted_content':'CIPHER_NOT_CHAT'},{'type':'tool_use','name':'exec','input':{'command':'pwd'}}]}, {'role':'user','content':[{'type':'tool_result','tool_use_id':'call','content':'/project'}]}]
        path = self.write('jcode',{'messages':messages,'id':'native-id','working_dir':'/project','model':'deepseek'})
        self.assertGreater(path.stat().st_size,8*1024*1024)
        entries = self.all_entries(self.request('jcode'))
        expected=''.join(message['content'][0]['text'] for message in messages[1:241])
        self.assertEqual(''.join(e['text'] for e in entries if e['kind']=='assistant'),expected)
        self.assertEqual([e['kind'] for e in entries[-4:]],['reasoning','reasoning','tool','tool'])
        self.assertNotIn('CIPHER_NOT_CHAT', json.dumps(entries))

    def test_claude_blocks_and_foreign_or_stale_cursors(self):
        rows=[{'type':'user','sessionId':'native-id','cwd':'/project','timestamp':'2026-10-30T12:00:00Z','message':{'role':'user','content':'Hello'}}, {'type':'assistant','sessionId':'native-id','message':{'role':'assistant','model':'claude','content':[{'type':'thinking','thinking':'Think'}, {'type':'text','text':'Answer'}, {'type':'tool_use','name':'read','input':{'file':'a'}}]}}, {'type':'user','sessionId':'native-id','message':{'role':'user','content':[{'type':'tool_result','tool_use_id':'id','content':'Result'}, {'type':'image','source':{'type':'base64','data':'PRIVATE_BLOB'}}]}}]
        self.write('claude',rows)
        page=history.run(self.request('claude'))
        self.assertEqual([e['kind'] for e in page['entries']],['user','reasoning','assistant','tool','tool','activity'])
        self.assertNotIn('PRIVATE_BLOB',json.dumps(page))
        self.assertIsNotNone(page['entries'][0]['at_ms'])
        with self.assertRaises(history.HistoryError):
            history.run(dict(self.request('claude'),workdir='/other'))
        with self.assertRaises(history.HistoryError):
            history.run(dict(self.request('claude'),snapshot=page['snapshot'],position='before',boundary=100))
        # Snapshot is bound to provider, id and cwd even when message text matches.
        self.write('jcode',{'id':'native-id','working_dir':'/project','messages':[]})
        with self.assertRaises(history.HistoryError):
            history.run(self.request('jcode',snapshot=page['snapshot']))

    def test_old_history_discovery_does_not_depend_on_recent_inventory(self):
        self.write('jcode',{'id':'native-id','working_dir':'/project','messages':[{'role':'user','content':'old'}]})
        for n in range(250):
            (self.home/'.jcode/sessions'/f'session-new-{n}.json').write_text('{}')
        self.assertEqual(history.run(self.request('jcode'))['entries'][0]['text'],'old')
        path=self.home/'.jcode/sessions/native-id.json'
        path.unlink()
        outside=self.home/'outside.json'
        outside.write_text('{}')
        path.symlink_to(outside)
        with self.assertRaises(history.HistoryError):
            history.run(self.request('jcode'))


if __name__ == '__main__':
    unittest.main()
