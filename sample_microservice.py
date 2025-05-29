#!/usr/bin/env python3
"""
Sample microservice demonstrating GC Sidecar integration
"""

import asyncio
import json
import tempfile
import os
from dataclasses import dataclass
from typing import Optional
import subprocess
import time

@dataclass
class GCLease:
    lease_id: str
    object_id: str
    service_id: str
    expires_at: str

class GCClient:
    def __init__(self, gc_host="localhost", gc_port="50051"):
        self.gc_host = gc_host
        self.gc_port = gc_port
        self.service_id = "sample-microservice"
    
    def create_lease(self, object_id: str, object_type: str, duration: int = 300) -> Optional[GCLease]:
        """Create a lease for an object"""
        cmd = [
            "grpcurl", "-plaintext", "-import-path", "proto", "-proto", "gc_service.proto",
            "-d", json.dumps({
                "object_id": object_id,
                "object_type": object_type,
                "service_id": self.service_id,
                "lease_duration_seconds": duration,
                "cleanup_config": {
                    "cleanup_http_endpoint": "http://localhost:8081/cleanup",
                    "max_retries": 3,
                    "retry_delay_seconds": 2
                }
            }),
            f"{self.gc_host}:{self.gc_port}",
            "distributed_gc.DistributedGCService/CreateLease"
        ]
        
        try:
            result = subprocess.run(cmd, capture_output=True, text=True, check=True)
            response = json.loads(result.stdout)
            
            if response.get("success"):
                return GCLease(
                    lease_id=response["leaseId"],
                    object_id=object_id,
                    service_id=self.service_id,
                    expires_at=response["expiresAt"]
                )
        except Exception as e:
            print(f"Failed to create lease: {e}")
        
        return None
    
    def renew_lease(self, lease: GCLease, duration: int = 300) -> bool:
        """Renew an existing lease"""
        cmd = [
            "grpcurl", "-plaintext", "-import-path", "proto", "-proto", "gc_service.proto",
            "-d", json.dumps({
                "lease_id": lease.lease_id,
                "service_id": self.service_id,
                "extend_duration_seconds": duration
            }),
            f"{self.gc_host}:{self.gc_port}",
            "distributed_gc.DistributedGCService/RenewLease"
        ]
        
        try:
            result = subprocess.run(cmd, capture_output=True, text=True, check=True)
            response = json.loads(result.stdout)
            return response.get("success", False)
        except Exception as e:
            print(f"Failed to renew lease: {e}")
            return False
    
    def release_lease(self, lease: GCLease) -> bool:
        """Release a lease early"""
        cmd = [
            "grpcurl", "-plaintext", "-import-path", "proto", "-proto", "gc_service.proto",
            "-d", json.dumps({
                "lease_id": lease.lease_id,
                "service_id": self.service_id
            }),
            f"{self.gc_host}:{self.gc_port}",
            "distributed_gc.DistributedGCService/ReleaseLease"
        ]
        
        try:
            result = subprocess.run(cmd, capture_output=True, text=True, check=True)
            response = json.loads(result.stdout)
            return response.get("success", False)
        except Exception as e:
            print(f"Failed to release lease: {e}")
            return False

class FileManager:
    """Example service that manages temporary files with GC integration"""
    
    def __init__(self):
        self.gc_client = GCClient()
        self.temp_files = {}
        self.leases = {}
    
    def create_temp_file(self, content: str, duration: int = 300) -> Optional[str]:
        """Create a temporary file with a GC lease"""
        
        # Create temporary file
        temp_file = tempfile.NamedTemporaryFile(mode='w', delete=False, suffix='.tmp')
        temp_file.write(content)
        temp_file.close()
        
        file_id = os.path.basename(temp_file.name)
        self.temp_files[file_id] = temp_file.name
        
        print(f"📁 Created temp file: {file_id} -> {temp_file.name}")
        
        # Create GC lease
        lease = self.gc_client.create_lease(
            object_id=file_id,
            object_type="TEMPORARY_FILE",
            duration=duration
        )
        
        if lease:
            self.leases[file_id] = lease
            print(f"🎫 Created lease: {lease.lease_id} for file: {file_id}")
            return file_id
        else:
            # Cleanup file if lease creation failed
            os.unlink(temp_file.name)
            del self.temp_files[file_id]
            print(f"❌ Failed to create lease for file: {file_id}")
            return None
    
    def read_file(self, file_id: str) -> Optional[str]:
        """Read a temporary file and renew its lease"""
        if file_id not in self.temp_files:
            return None
        
        # Renew lease before accessing
        if file_id in self.leases:
            if self.gc_client.renew_lease(self.leases[file_id]):
                print(f"🔄 Renewed lease for file: {file_id}")
            else:
                print(f"⚠️ Failed to renew lease for file: {file_id}")
        
        try:
            with open(self.temp_files[file_id], 'r') as f:
                return f.read()
        except FileNotFoundError:
            print(f"📁 File not found: {file_id}")
            return None
    
    def delete_file(self, file_id: str) -> bool:
        """Explicitly delete a file and release its lease"""
        if file_id not in self.temp_files:
            return False
        
        # Release lease first
        if file_id in self.leases:
            if self.gc_client.release_lease(self.leases[file_id]):
                print(f"🎫 Released lease for file: {file_id}")
            del self.leases[file_id]
        
        # Delete file
        try:
            os.unlink(self.temp_files[file_id])
            del self.temp_files[file_id]
            print(f"🗑️ Deleted file: {file_id}")
            return True
        except FileNotFoundError:
            print(f"📁 File already deleted: {file_id}")
            return True
    
    def cleanup_file(self, file_id: str):
        """Cleanup callback for expired leases"""
        if file_id in self.temp_files:
            try:
                os.unlink(self.temp_files[file_id])
                del self.temp_files[file_id]
                print(f"🧹 GC cleaned up file: {file_id}")
            except FileNotFoundError:
                print(f"🧹 GC cleanup - file already gone: {file_id}")
        
        if file_id in self.leases:
            del self.leases[file_id]

# Cleanup server to handle GC callbacks
from http.server import HTTPServer, BaseHTTPRequestHandler
import threading

class CleanupServer:
    def __init__(self, file_manager: FileManager, port: int = 8081):
        self.file_manager = file_manager
        self.port = port
        self.server = None
        self.thread = None
    
    def start(self):
        class CleanupHandler(BaseHTTPRequestHandler):
            def __init__(self, *args, file_manager=None, **kwargs):
                self.file_manager = file_manager
                super().__init__(*args, **kwargs)
            
            def do_POST(self):
                try:
                    content_length = int(self.headers['Content-Length'])
                    post_data = self.rfile.read(content_length)
                    cleanup_request = json.loads(post_data)
                    
                    object_id = cleanup_request.get('object_id')
                    print(f"🧹 Cleanup request for: {object_id}")
                    
                    # Call cleanup on the file manager
                    self.file_manager.cleanup_file(object_id)
                    
                    self.send_response(200)
                    self.send_header('Content-type', 'application/json')
                    self.end_headers()
                    self.wfile.write(json.dumps({'success': True}).encode())
                    
                except Exception as e:
                    print(f"❌ Cleanup error: {e}")
                    self.send_response(500)
                    self.send_header('Content-type', 'application/json')
                    self.end_headers()
                    self.wfile.write(json.dumps({'error': str(e)}).encode())
            
            def log_message(self, format, *args):
                pass  # Suppress HTTP logs
        
        # Create handler with file_manager reference
        handler = lambda *args, **kwargs: CleanupHandler(*args, file_manager=self.file_manager, **kwargs)
        
        self.server = HTTPServer(('localhost', self.port), handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        print(f"🚀 Cleanup server started on port {self.port}")
    
    def stop(self):
        if self.server:
            self.server.shutdown()

def main():
    print("🎯 Starting Sample Microservice with GC Integration")
    print("=" * 60)
    
    # Start file manager and cleanup server
    file_manager = FileManager()
    cleanup_server = CleanupServer(file_manager)
    cleanup_server.start()
    
    try:
        # Demo workflow
        print("\n📝 Creating temporary files...")
        
        # Create files with different lifespans
        file1 = file_manager.create_temp_file("Hello, World!", duration=30)  # 30 seconds
        file2 = file_manager.create_temp_file("Temporary data", duration=60)  # 60 seconds
        file3 = file_manager.create_temp_file("Long-lived content", duration=300)  # 5 minutes
        
        if not all([file1, file2, file3]):
            print("❌ Failed to create files")
            return
        
        print(f"\n📖 Reading files...")
        print(f"File 1 content: {file_manager.read_file(file1)}")
        print(f"File 2 content: {file_manager.read_file(file2)}")
        
        print(f"\n⏰ Waiting for lease expiration...")
        print("Watch the GC sidecar logs for cleanup operations...")
        print("Files should be automatically cleaned up when leases expire")
        
        # Keep the demo running
        print("\n🔄 Running demo loop (Ctrl+C to stop)...")
        while True:
            time.sleep(10)
            
            # Try to read files periodically
            content = file_manager.read_file(file2)
            if content:
                print(f"📖 File 2 still accessible: {len(content)} chars")
            else:
                print("🧹 File 2 has been cleaned up")
            
            # Explicitly delete file3 after some time
            if time.time() % 60 < 10:  # Every minute for 10 seconds
                print(f"🗑️ Explicitly deleting file 3...")
                file_manager.delete_file(file3)
    
    except KeyboardInterrupt:
        print("\n🛑 Stopping demo...")
    finally:
        cleanup_server.stop()
        print("✅ Demo completed")

if __name__ == "__main__":
    main()