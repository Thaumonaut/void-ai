import http.server, time, os, urllib.parse
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        u=urllib.parse.urlparse(self.path); q=urllib.parse.parse_qs(u.query)
        rate=float(q.get('rate',[0])[0])*1024      # KB/s -> bytes/s (0=unlimited)
        lat=float(q.get('lat',[0])[0])/1000.0       # ms -> s initial latency
        path=u.path.lstrip('/')
        if not os.path.isfile(path): self.send_error(404); return
        data=open(path,'rb').read()
        self.send_response(200)
        self.send_header('Content-Type','application/octet-stream')
        self.send_header('Content-Length',str(len(data))); self.end_headers()
        if lat>0: time.sleep(lat)
        if rate<=0: self.wfile.write(data)
        else:
            chunk=max(1,int(rate/20))
            for i in range(0,len(data),chunk):
                self.wfile.write(data[i:i+chunk]); self.wfile.flush()
                time.sleep(chunk/rate)
    def log_message(self,*a): pass
http.server.HTTPServer(('0.0.0.0',8898),H).serve_forever()
