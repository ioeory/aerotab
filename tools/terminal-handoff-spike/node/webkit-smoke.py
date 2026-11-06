"""Minimal ctypes GTK/WebKit fixture host using installed shared libraries.
Real rAF + Canvas readback, not physical presentation or production IPC.
"""
import ctypes as C
import http.server
import json
import pathlib
import subprocess
import threading
root=pathlib.Path(__file__).resolve().parent
frames=[json.loads(line) for line in subprocess.check_output([str(root.parent/'target/debug/view-fixture')],text=True).splitlines()]
class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self,*args,**kwargs):super().__init__(*args,directory=str(root),**kwargs)
    def do_GET(self):
        if self.path=='/frames':
            data=json.dumps(frames).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(data)
        else:super().do_GET()
    def log_message(self,*args):pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
gtk=C.CDLL('libgtk-3.so.0');web=C.CDLL('libwebkit2gtk-4.1.so.0');gobj=C.CDLL('libgobject-2.0.so.0');glib=C.CDLL('libglib-2.0.so.0');jsc=C.CDLL('libjavascriptcoregtk-4.1.so.0')
def fn(lib,name,restype,args):
    f=getattr(lib,name);f.restype=restype;f.argtypes=args;return f
ptr=C.c_void_p
fn(gtk,'gtk_init',None,[ptr,ptr])(None,None)
window=fn(gtk,'gtk_window_new',ptr,[C.c_int])(0)
view=fn(web,'webkit_web_view_new',ptr,[])()
fn(gtk,'gtk_container_add',None,[ptr,ptr])(window,view)
manager=fn(web,'webkit_web_view_get_user_content_manager',ptr,[ptr])(view)
fn(web,'webkit_user_content_manager_register_script_message_handler',C.c_int,[ptr,C.c_char_p])(manager,b'evidence')
result={'ok':False,'error':'WebKit smoke timeout'}
CALLBACK=C.CFUNCTYPE(None,ptr,ptr,ptr)
@CALLBACK
def received(_manager,message,_user):
    value=fn(web,'webkit_javascript_result_get_js_value',ptr,[ptr])(message)
    string=fn(jsc,'jsc_value_to_string',ptr,[ptr])(value)
    result.clear();result.update(json.loads(C.string_at(string).decode()))
    fn(glib,'g_free',None,[ptr])(string)
    gtk.gtk_main_quit()
fn(gobj,'g_signal_connect_data',C.c_ulong,[ptr,C.c_char_p,CALLBACK,ptr,ptr,C.c_int])(manager,b'script-message-received::evidence',received,None,None,0)
TIMER=C.CFUNCTYPE(C.c_int,ptr)
@TIMER
def timeout(_):gtk.gtk_main_quit();return 0
fn(glib,'g_timeout_add',C.c_uint,[C.c_uint,TIMER,ptr])(20000,timeout,None)
fn(gtk,'gtk_widget_show_all',None,[ptr])(window)
url=f'http://127.0.0.1:{server.server_port}/view-smoke.html'.encode()
fn(web,'webkit_web_view_load_uri',None,[ptr,C.c_char_p])(view,url)
gtk.gtk_main();server.shutdown()
print(json.dumps(result));raise SystemExit(0 if result.get('ok') else 1)
