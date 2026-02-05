
#!/bin/bash

cat page*.txt | tr -cd 'A-Za-z0-9+/' | base64 -d > /tmp/decoded.jpg
display /tmp/decoded.jpg &
